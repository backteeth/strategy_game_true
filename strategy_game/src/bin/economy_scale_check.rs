//! P32-024: 新ステート経済の規模測定ハーネスのコマンド行(`economy::scale_harness`)。
//!
//! 通常のゲーム起動や既存CLIには影響しない独立のバイナリ。測定は release ビルドで行うこと。
//! `--series` は、ケースごとに自分自身を別プロセスで逐次起動し(小さい順)、成功以外の結果が出たら以降のケースへ進まない。

use std::io::Read as _;
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};

use strategy_game::economy::map_session::load_buildings;
use strategy_game::economy::scale_harness::{
    Command, Paths, ScaleSpec, USAGE, WIDE_CASE, fresh_temp_base, parse_args, run_case,
};
use strategy_game::hexmap::data::load_from_path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Err(m) => {
            eprintln!("error: {m}\n\n{USAGE}");
            std::process::exit(64);
        }
        Ok(Command::Help) => println!("{USAGE}"),
        Ok(Command::Case { spec, paths }) => std::process::exit(run_single(&spec, &paths)),
        Ok(Command::Series { sizes, wide, template, paths }) => std::process::exit(run_series(&sizes, wide, &template, &paths)),
    }
}

fn print_environment() {
    println!(
        "環境: os={} arch={} family={} build={} cpus(available_parallelism)={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::consts::FAMILY,
        if cfg!(debug_assertions) { "debug(測定には不適)" } else { "release" },
        std::thread::available_parallelism().map_or("不明".to_string(), |n| n.get().to_string())
    );
}

fn run_single(spec: &ScaleSpec, paths: &Paths) -> i32 {
    print_environment();
    let map = match load_from_path(&paths.map) {
        Ok(m) => m,
        Err(e) => {
            println!("判定: 実行不能\n  地図を読み込めない: {e}");
            return 4;
        }
    };
    let buildings = match load_buildings(&paths.buildings) {
        Ok(b) => b,
        Err(e) => {
            println!("判定: 実行不能\n  建物定義を読み込めない: {e}");
            return 4;
        }
    };
    let base = fresh_temp_base(&format!("s{}", spec.states));
    let result = run_case(&map, &buildings, spec, &base);
    // 空なら消える(自分が作ったディレクトリだけ。中身があれば残る)。
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(&base);
    print!("{}", result.render());
    println!("{}", result.row());
    result.status.exit_code()
}

struct Row {
    label: String,
    outcome: String,
    line: Option<String>,
}

fn run_series(sizes: &[u32], wide: bool, template: &ScaleSpec, paths: &Paths) -> i32 {
    print_environment();
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot find the executable: {e}");
            return 4;
        }
    };
    let mut cases: Vec<(String, u32, u32)> = sizes.iter().map(|&n| ("主系列".to_string(), n, template.countries)).collect();
    if wide {
        cases.push(("別条件".to_string(), WIDE_CASE.0, WIDE_CASE.1));
    }
    let hard_limit = template.time_limit + Duration::from_secs(30);
    let mut rows: Vec<Row> = Vec::new();
    let mut worst = 0;
    let mut stopped = false;
    let mut main_ok = true;
    for (kind, states, countries) in cases {
        let label = format!("{kind} states={states} countries={countries}");
        if stopped {
            rows.push(Row { label, outcome: "未実行(前のケースが成功しなかったため進まない)".into(), line: None });
            continue;
        }
        if kind == "別条件" && !main_ok {
            rows.push(Row { label, outcome: "未実行(主系列が成功しなかったため)".into(), line: None });
            continue;
        }
        let mut cmd = Proc::new(&exe);
        cmd.args(["--states", &states.to_string(), "--countries", &countries.to_string()])
            .args(["--provinces-per-state", &template.provinces_per_state.to_string()])
            .args(["--seed", &template.seed.to_string(), "--days", &template.days.to_string()])
            .args(["--split", &template.split_day.to_string(), "--runs", &template.runs.to_string()])
            .args(["--time-limit-secs", &template.time_limit.as_secs().to_string()])
            .args(["--map", &paths.map, "--buildings", &paths.buildings]);
        if !template.scripted_orders {
            cmd.arg("--no-script");
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
        println!("\n--- {label} を別プロセスで実行 ---");
        let started = Instant::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                rows.push(Row { label, outcome: format!("実行不能(起動失敗: {e})"), line: None });
                worst = worst.max(4);
                stopped = true;
                continue;
            }
        };
        // 子の出力は別スレッドで読み(パイプ詰まり防止)、ここでは上限まで待つ。
        let mut out = child.stdout.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            s
        });
        let mut killed = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) => {
                    if started.elapsed() > hard_limit {
                        let _ = child.kill();
                        let _ = child.wait();
                        killed = true;
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break None,
            }
        };
        let text = reader.join().unwrap_or_default();
        print!("{text}");
        let line = text.lines().find(|l| l.starts_with("ROW\t")).map(str::to_string);
        let (outcome, code) = if killed {
            ("時間超過(上限+猶予を超えたため強制終了。未完了)".to_string(), 3)
        } else {
            match status.and_then(|s| s.code()) {
                Some(0) => ("成功".to_string(), 0),
                Some(2) => ("正しさの失敗".to_string(), 2),
                Some(3) => ("時間超過(未完了)".to_string(), 3),
                Some(4) => ("実行不能".to_string(), 4),
                Some(c) => (format!("異常終了(終了コード {c})"), 5),
                None => ("異常終了(シグナル等。終了コードなし)".to_string(), 5),
            }
        };
        println!("--- {label}: {outcome}(実時間 {:.1}s) ---", started.elapsed().as_secs_f64());
        if code != 0 {
            worst = worst.max(code);
            stopped = true;
            if kind == "主系列" {
                main_ok = false;
            }
        }
        rows.push(Row { label, outcome, line });
    }
    println!("\n=== 要約(進行は連続実行#1。時間はms。計測範囲は各ケースの詳細を参照) ===");
    println!("条件\t結果\t進行合計\t1日平均\t月次日平均\t通常日平均\t保存\t復元\tファイルB\t処理日数\t比較日数\t決定性\tケース全体ms");
    for r in &rows {
        match &r.line {
            Some(l) => {
                // ROW states countries status adv mean monthly normal save restore bytes days compared det total
                let f: Vec<&str> = l.split('\t').collect();
                if f.len() >= 15 {
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        r.label, r.outcome, f[4], f[5], f[6], f[7], f[8], f[9], f[10], f[11], f[12], f[13], f[14]
                    );
                } else {
                    println!("{}\t{}\t(行の形式不明)", r.label, r.outcome);
                }
            }
            None => println!("{}\t{}", r.label, r.outcome),
        }
    }
    if worst != 0 {
        println!("\n成功以外の結果があります。全規模成功とは扱わないでください。");
    }
    worst
}
