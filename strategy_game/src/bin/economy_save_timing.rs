//! P32-026: 保存・復元の工程別計時の測定コマンド(`economy::save_timing_bench`)。
//!
//! 通常のゲーム起動や既存CLIには影響しない独立のバイナリ。測定は release ビルドで行うこと。
//! 保存先は試行ごとのOSの一時ディレクトリで、`saves/` へは書かない。計時の記録は標準出力にだけ出し、ファイルは作らない。
//!
//! 単独実行: `economy_save_timing --states 1000 --countries 3 --mode on --trials 3`
//! 一連の測定: `economy_save_timing --series` は、ケース×モードごとに自分自身を別プロセスで逐次起動する
//! (各ケースは上限+30秒で強制終了し、未完了として記録する。成功しなかったら以降へ進まない)。

use std::io::Read as _;
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};

use strategy_game::economy::map_session::load_buildings;
use strategy_game::economy::save_timing_bench::{BenchSpec, run_bench_case};
use strategy_game::economy::scale_harness::{CaseStatus, ScaleSpec, fresh_temp_base};
use strategy_game::hexmap::data::load_from_path;

const USAGE: &str = "\
usage:
  economy_save_timing --states N --countries C [--mode on|off] [--trials T] [--time-limit-secs S] [--seed X]
                      [--provinces-per-state P] [--map PATH] [--buildings PATH]
  economy_save_timing --series [--time-limit-secs S] [--trials T]
測定ケース(--series): 100x3 / 300x3 / 1000x3 / 1000x100 を、計時なし(off)・計時あり(on)・計時あり・計時なしの順に別プロセスで実行する。";

struct Args {
    spec: BenchSpec,
    map: String,
    buildings: String,
    series: bool,
    cases: Vec<(u32, u32)>,
    modes: Vec<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut states = 100u32;
    let mut countries = 3u32;
    let mut timed = false;
    let mut trials = 3u32;
    let mut limit = 120u64;
    let mut seed = None;
    let mut pps = None;
    let mut map = "assets/data/hex_atlas_map.json".to_string();
    let mut buildings = "assets/data/buildings.ron".to_string();
    let mut series = false;
    let mut cases = vec![(100u32, 3u32), (300, 3), (1000, 3), (1000, 100)];
    let mut modes: Vec<String> = ["off", "on", "on", "off"].iter().map(|x| x.to_string()).collect();
    let mut i = 0;
    let value = |i: usize, name: &str| args.get(i + 1).cloned().ok_or_else(|| format!("{name} needs a value"));
    let num = |s: String, name: &str| s.parse::<u64>().map_err(|_| format!("{name}: \"{s}\" is not a number"));
    while i < args.len() {
        match args[i].as_str() {
            "--states" => states = num(value(i, "--states")?, "--states")? as u32,
            "--countries" => countries = num(value(i, "--countries")?, "--countries")? as u32,
            "--trials" => trials = num(value(i, "--trials")?, "--trials")? as u32,
            "--time-limit-secs" => limit = num(value(i, "--time-limit-secs")?, "--time-limit-secs")?,
            "--seed" => seed = Some(num(value(i, "--seed")?, "--seed")?),
            "--provinces-per-state" => pps = Some(num(value(i, "--provinces-per-state")?, "--provinces-per-state")? as u32),
            "--mode" => {
                timed = match value(i, "--mode")?.as_str() {
                    "on" => true,
                    "off" => false,
                    other => return Err(format!("--mode: \"{other}\" is not on/off")),
                }
            }
            "--map" => map = value(i, "--map")?,
            "--buildings" => buildings = value(i, "--buildings")?,
            "--cases" => {
                cases = value(i, "--cases")?
                    .split(',')
                    .map(|c| {
                        let (x, y) = c.split_once('x').ok_or_else(|| format!("--cases: {c} is not NxC"))?;
                        Ok((x.parse::<u32>().map_err(|e| e.to_string())?, y.parse::<u32>().map_err(|e| e.to_string())?))
                    })
                    .collect::<Result<_, String>>()?;
            }
            "--modes" => modes = value(i, "--modes")?.split(',').map(|x| x.to_string()).collect(),
            "--series" => {
                series = true;
                i += 1;
                continue;
            }
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    let mut scale = ScaleSpec::new(states, countries);
    scale.time_limit = Duration::from_secs(limit);
    if let Some(s) = seed {
        scale.seed = s;
    }
    if let Some(p) = pps {
        scale.provinces_per_state = p;
    }
    Ok(Args { spec: BenchSpec { scale, timed, trials }, map, buildings, series, cases, modes })
}

fn print_environment() {
    println!(
        "ENV os={} arch={} build={} cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "debug(not for measurement)" } else { "release" },
        std::thread::available_parallelism().map_or("unknown".to_string(), |n| n.get().to_string())
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&args) {
        Ok(a) => a,
        Err(m) => {
            if !m.is_empty() {
                eprintln!("error: {m}\n");
            }
            eprintln!("{USAGE}");
            std::process::exit(if m.is_empty() { 0 } else { 64 });
        }
    };
    if a.series {
        std::process::exit(series(&a));
    }
    std::process::exit(single(&a));
}

fn single(a: &Args) -> i32 {
    print_environment();
    let map = match load_from_path(&a.map) {
        Ok(m) => m,
        Err(e) => {
            println!("INCOMPLETE unrunnable=cannot load the map: {e}");
            return 4;
        }
    };
    let buildings = match load_buildings(&a.buildings) {
        Ok(b) => b,
        Err(e) => {
            println!("INCOMPLETE unrunnable=cannot load the buildings: {e}");
            return 4;
        }
    };
    let base = fresh_temp_base(&format!("p26_s{}", a.spec.scale.states));
    let r = run_bench_case(&map, &buildings, &a.spec, &base);
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(&base);
    print!("{}", r.render());
    match r.status {
        CaseStatus::Success => 0,
        CaseStatus::CorrectnessFailure(_) => 2,
        CaseStatus::TimedOut { .. } => 3,
        CaseStatus::Unrunnable(_) => 4,
    }
}

fn series(a: &Args) -> i32 {
    print_environment();
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find the executable: {e}");
            return 4;
        }
    };
    let cases = a.cases.clone();
    // 順序の影響を見るため、既定は off → on → on → off の順(同じ入力・同じ状態・同じビルド)。
    let modes: Vec<&str> = a.modes.iter().map(|x| x.as_str()).collect();
    let hard = Duration::from_secs(a.spec.scale.time_limit.as_secs() + 30);
    let mut worst = 0;
    'outer: for (states, countries) in cases {
        for (k, mode) in modes.iter().enumerate() {
            println!("\n=== RUN states={states} countries={countries} mode={mode} pass={} ===", k + 1);
            let mut cmd = Proc::new(&exe);
            cmd.args(["--states", &states.to_string(), "--countries", &countries.to_string(), "--mode", mode])
                .args(["--trials", &a.spec.trials.to_string(), "--time-limit-secs", &a.spec.scale.time_limit.as_secs().to_string()])
                .args(["--map", &a.map, "--buildings", &a.buildings])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
            let started = Instant::now();
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    println!("INCOMPLETE case={states}x{countries} mode={mode} unrunnable=spawn failed: {e}");
                    worst = 4;
                    break 'outer;
                }
            };
            let mut out = child.stdout.take().expect("piped");
            let reader = std::thread::spawn(move || {
                let mut s = String::new();
                let _ = out.read_to_string(&mut s);
                s
            });
            let mut killed = false;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(s)) => break Some(s),
                    Ok(None) => {
                        if started.elapsed() > hard {
                            let _ = child.kill();
                            let _ = child.wait();
                            killed = true;
                            break None;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break None,
                }
            };
            let text = reader.join().unwrap_or_default();
            print!("{text}");
            let code = if killed { 3 } else { status.and_then(|s| s.code()).unwrap_or(5) };
            println!("EXIT case={states}x{countries} mode={mode} pass={} code={code} killed={killed} wall_s={:.1}", k + 1, started.elapsed().as_secs_f64());
            if code != 0 {
                worst = code;
                println!("STOP: the case did not finish successfully; later cases are not run (the same failure is not repeated)");
                break 'outer;
            }
        }
    }
    worst
}
