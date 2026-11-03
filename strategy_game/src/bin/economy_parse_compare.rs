//! P32-027: 保存文章のヘッダー解析の比較測定(**測定専用**。通常のゲーム起動・保存・復元の動作は変えない)。
//!
//! 製品の `map_economy_file::decode_timed`(ヘッダー解析と本体解析を別の工程として計る。製品の `decode` と同じ処理)と、
//! 測定専用の部品(`economy::parse_probe`)で、同じ文章・同じ条件のまま解析の時間を比べる。測定は release ビルドで行うこと。
//! 保存の生成は製品の `save`(一時ディレクトリ)で、ファイルの読込・結果の破棄・出力は解析の区間に含めない。
//!
//! 使い方(`cargo run -j 1 --release --bin economy_parse_compare -- ...`):
//!   --section trials   --states N --countries C [--trials T]  製品の保存文章(ステート数N)を使い回し、同一プロセス内で繰り返す
//!   --section variants --states N --countries C [--trials T]  同じ保存の内容を、状態だけ/国家だけ/状態の前半だけにした妥当な文章
//!   --section shapes   [--shape NAME] [--sizes-kb 125,250,500,1000] [--trials T]  kind・版を保ち、payloadの形と量だけを変えた妥当なRON
//! 1プロセス(1条件)の上限は `--time-limit-secs`(既定120秒)。超えたら未完了として記録して終了する。

use std::time::{Duration, Instant};

use strategy_game::economy::map_economy_file::{decode, decode_timed, encode};
use strategy_game::economy::map_session::load_buildings;
use strategy_game::economy::parse_probe::{Shape, fixture, parse_header_replica, parse_ignored_any, process_memory, text_stats};
use strategy_game::economy::phase_timing::Timing;
use strategy_game::economy::save_timing_bench::build_saved_text;
use strategy_game::economy::scale_harness::{ScaleSpec, fresh_temp_base};
use strategy_game::hexmap::data::load_from_path;

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn mem_line(point: &str, trial: u32) {
    match process_memory() {
        Some(m) => println!(
            "MEM point={point} trial={trial} working_set_bytes={} private_bytes={} peak_working_set_since_process_start_bytes={}",
            m.working_set, m.private_bytes, m.peak_working_set
        ),
        None => println!("MEM point={point} trial={trial} unavailable(not measured)"),
    }
}

fn stats_line(label: &str, text: &str) {
    let s = text_stats(text);
    println!(
        "INPUT label={label} bytes={} lines={} parens={} brackets={} max_depth={} fields={}",
        s.bytes, s.lines, s.parens, s.brackets, s.max_depth, s.fields
    );
}

struct Args {
    section: String,
    states: u32,
    countries: u32,
    trials: u32,
    limit: Duration,
    shape: Option<String>,
    sizes_kb: Vec<usize>,
    map: String,
    buildings: String,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        section: "trials".into(),
        states: 1000,
        countries: 3,
        trials: 3,
        limit: Duration::from_secs(120),
        shape: None,
        sizes_kb: vec![125, 250, 500, 1000],
        map: "assets/data/hex_atlas_map.json".into(),
        buildings: "assets/data/buildings.ron".into(),
    };
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]))?;
        let n = |s: &str| s.parse::<u64>().map_err(|_| format!("{}: \"{s}\" is not a number", args[i]));
        match args[i].as_str() {
            "--section" => a.section = v,
            "--states" => a.states = n(&v)? as u32,
            "--countries" => a.countries = n(&v)? as u32,
            "--trials" => a.trials = n(&v)? as u32,
            "--time-limit-secs" => a.limit = Duration::from_secs(n(&v)?),
            "--shape" => a.shape = Some(v),
            "--sizes-kb" => {
                a.sizes_kb = v.split(',').map(|x| x.parse::<usize>().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
            }
            "--map" => a.map = v,
            "--buildings" => a.buildings = v,
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    Ok(a)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = match parse(&args) {
        Ok(a) => a,
        Err(m) => {
            eprintln!("error: {m}\nusage: economy_parse_compare --section trials|variants|shapes [--states N --countries C] [--trials T] [--shape NAME] [--sizes-kb 125,250,500,1000] [--time-limit-secs S]");
            std::process::exit(64);
        }
    };
    println!(
        "ENV os={} arch={} build={} cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "debug(not for measurement)" } else { "release" },
        std::thread::available_parallelism().map_or("unknown".to_string(), |n| n.get().to_string())
    );
    std::process::exit(run(&a));
}

fn run(a: &Args) -> i32 {
    let started = Instant::now();
    let over = |what: &str| -> bool {
        if started.elapsed() > a.limit {
            println!("INCOMPLETE section={} time_limit_exceeded_before={what}", a.section);
            true
        } else {
            false
        }
    };
    match a.section.as_str() {
        "shapes" => shapes(a, &over),
        "trials" | "variants" => from_product_save(a, &over),
        other => {
            println!("INCOMPLETE unrunnable=unknown section {other}");
            64
        }
    }
}

fn shapes(a: &Args, over: &dyn Fn(&str) -> bool) -> i32 {
    let list: Vec<Shape> = match &a.shape {
        Some(n) => match Shape::parse(n) {
            Some(s) => vec![s],
            None => {
                println!("INCOMPLETE unrunnable=unknown shape {n}");
                return 4;
            }
        },
        None => Shape::ALL.to_vec(),
    };
    for shape in list {
        for kb in &a.sizes_kb {
            let text = fixture(shape, kb * 1000);
            // 妥当なRONであることの確認(計時の外。値として読む別の読み方)
            if let Err(e) = ron::from_str::<ron::Value>(&text) {
                println!("INCOMPLETE unrunnable=fixture {} {kb}KB is not valid RON: {e}", shape.name());
                return 4;
            }
            let st = text_stats(&text);
            for trial in 1..=a.trials {
                if over(&format!("{} {kb}KB trial {trial}", shape.name())) {
                    return 3;
                }
                let t = Instant::now();
                let h = parse_header_replica(&text);
                let hdr = ms(t.elapsed());
                let t = Instant::now();
                let ig = parse_ignored_any(&text);
                let ign = ms(t.elapsed());
                let ok = matches!(&h, Ok((Some(k), Some(2))) if k == "map_economy_state") && ig.is_ok();
                println!(
                    "SHAPE shape={} target_kb={kb} bytes={} parens={} brackets={} max_depth={} fields={} trial={trial} header_ms={hdr:.3} ignored_any_ms={ign:.3} parsed_ok={ok}",
                    shape.name(),
                    st.bytes,
                    st.parens,
                    st.brackets,
                    st.max_depth,
                    st.fields
                );
            }
        }
    }
    0
}

fn from_product_save(a: &Args, over: &dyn Fn(&str) -> bool) -> i32 {
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
    let mut scale = ScaleSpec::new(a.states, a.countries);
    scale.time_limit = a.limit;
    let base = fresh_temp_base(&format!("p27_s{}", a.states));
    let saved = match build_saved_text(&map, &buildings, &scale, &base) {
        Ok(s) => s,
        Err(e) => {
            println!("INCOMPLETE unrunnable={e}");
            return 4;
        }
    };
    let text = saved.text;
    let decoded = match decode(&text) {
        Ok(d) => d,
        Err(e) => {
            println!("INCOMPLETE unrunnable=the product save does not decode: {e:?}");
            return 4;
        }
    };
    let building_entries: usize = decoded.snapshot.states.iter().map(|s| s.buildings.len()).sum();
    println!(
        "CASE states={} countries={} queued_orders={} state_entries={} building_entries={} country_entries={}",
        saved.states,
        saved.countries,
        saved.queued_orders,
        decoded.snapshot.states.len(),
        building_entries,
        decoded.snapshot.countries.len()
    );
    if a.section == "trials" {
        stats_line("product_save_text", &text);
        for trial in 1..=a.trials {
            if over(&format!("trial {trial}")) {
                return 3;
            }
            mem_line("before_trial", trial);
            // 区間: メモリ上の同じ文章(使い回し)の解析だけ。ファイル読込・結果の破棄・出力は含まない。
            let mut tm = Timing::on();
            let t = Instant::now();
            let res = decode_timed(&text, &mut tm, ("header", "body"));
            let wall = ms(t.elapsed());
            let ok = res.is_ok();
            drop(res); // 解析結果の破棄(計時の外)
            let hdr = tm.span("header").map_or(f64::NAN, |s| ms(s.total));
            let body = tm.span("body").map_or(f64::NAN, |s| ms(s.total));
            let t = Instant::now();
            let _ = parse_header_replica(&text);
            let rep = ms(t.elapsed());
            let t = Instant::now();
            let _ = parse_ignored_any(&text);
            let ign = ms(t.elapsed());
            // 対照: 解析と無関係な文章の線形走査(プロセス全体が遅くなっているかを見る)
            let t = Instant::now();
            let st = text_stats(&text);
            let ctl = ms(t.elapsed());
            println!(
                "PARSE trial={trial} product_decode_wall_ms={wall:.3} product_header_ms={hdr:.3} product_body_ms={body:.3} replica_header_ms={rep:.3} ignored_any_full_ms={ign:.3} control_text_scan_ms={ctl:.3} bytes={} ok={ok}",
                st.bytes
            );
            mem_line("after_trial", trial);
        }
        return 0;
    }
    // variants: 同じ保存の内容から、製品の encode で書き出した妥当な別の文章
    let identity = decoded.identity.clone();
    let mut variants = vec![("full", decoded.snapshot.clone())];
    let mut v = decoded.snapshot.clone();
    v.countries.clear();
    variants.push(("states_only", v));
    let mut v = decoded.snapshot.clone();
    v.states.clear();
    variants.push(("countries_only", v));
    let mut v = decoded.snapshot.clone();
    let half = v.states.len() / 2;
    v.states.truncate(half);
    variants.push(("states_first_half", v));
    for (name, snap) in variants {
        let vt = match encode(&snap, &identity) {
            Ok(t) => t,
            Err(e) => {
                println!("INCOMPLETE unrunnable=cannot encode variant {name}: {e:?}");
                return 4;
            }
        };
        stats_line(name, &vt);
        for trial in 1..=a.trials {
            if over(&format!("variant {name} trial {trial}")) {
                return 3;
            }
            let mut tm = Timing::on();
            let res = decode_timed(&vt, &mut tm, ("header", "body"));
            let ok = res.is_ok();
            drop(res);
            let hdr = tm.span("header").map_or(f64::NAN, |s| ms(s.total));
            let body = tm.span("body").map_or(f64::NAN, |s| ms(s.total));
            let t = Instant::now();
            let _ = parse_header_replica(&vt);
            let rep = ms(t.elapsed());
            println!("VARIANT name={name} trial={trial} product_header_ms={hdr:.3} product_body_ms={body:.3} replica_header_ms={rep:.3} ok={ok}");
        }
    }
    0
}
