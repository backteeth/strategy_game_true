//! P32-024: 規模測定ハーネス(`economy::scale_harness`)の自動検証。
//! ここでは**小さいケース**だけを使う(大規模測定は `economy_scale_check` を手動で実行する。通常のテストでは走らない)。
//! 測定用の保存はOSの一時ディレクトリだけに作り、`saves/` には書かない。

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use strategy_game::building::data::BuildingRegistry;
use strategy_game::economy::map_session::load_buildings;
use strategy_game::economy::scale_harness::{
    CaseStatus, Command, ScaleError, ScaleSpec, Script, assemble, build_world, date_after, fresh_temp_base, generate,
    parse_args, run_case, START,
};
use strategy_game::hexmap::data::{HexMap, load_from_path};

fn map() -> &'static HexMap {
    static M: OnceLock<HexMap> = OnceLock::new();
    M.get_or_init(|| load_from_path("assets/data/hex_atlas_map.json").unwrap())
}

fn buildings() -> &'static BuildingRegistry {
    static B: OnceLock<BuildingRegistry> = OnceLock::new();
    B.get_or_init(|| load_buildings("assets/data/buildings.ron").unwrap())
}

fn small(states: u32, countries: u32, days: u32, split: u32) -> ScaleSpec {
    let mut s = ScaleSpec::new(states, countries);
    s.days = days;
    s.split_day = split;
    s
}

fn run(spec: &ScaleSpec) -> strategy_game::economy::scale_harness::CaseResult {
    let base = fresh_temp_base("test");
    let r = run_case(map(), buildings(), spec, &base);
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(&base);
    r
}

// ---------------------------------------------------------------- 入力の生成

#[test]
fn the_requested_number_of_states_and_countries_is_generated() {
    for (n, c) in [(1, 1), (3, 3), (12, 4), (30, 7), (30, 30)] {
        let input = generate(&ScaleSpec::new(n, c), map()).unwrap();
        assert_eq!(input.layout.states.len(), n as usize);
        assert_eq!(input.scenario.states.len(), n as usize);
        assert_eq!(input.scenario.countries.len(), c as usize);
        assert_eq!(input.summary.states, n as usize);
        assert_eq!(input.summary.countries, c as usize);
        // 各国家が担当ステートを持つ。全ステートがちょうど1国家に属する。
        let mut owned: Vec<u32> = input.scenario.countries.iter().flat_map(|k| k.states.clone()).collect();
        assert!(input.scenario.countries.iter().all(|k| !k.states.is_empty()));
        owned.sort_unstable();
        let mut all: Vec<u32> = input.layout.states.iter().map(|s| s.id).collect();
        all.sort_unstable();
        assert_eq!(owned, all, "n={n} c={c}");
        // 1国家あたりのステート数は ⌊n/c⌋ 以上。
        assert!(input.summary.states_per_country_min >= (n / c) as usize);
        // 各ステートは指定数のプロヴィンスを持つ。
        assert!(input.layout.states.iter().all(|s| s.provinces.len() == 6));
        assert_eq!(input.table.provinces.len(), n as usize * 6);
        // 手動国家は1つだけ(ラベル1)、他はAI。
        assert!(!input.scenario.countries[0].ai_controlled);
        assert!(input.scenario.countries.iter().skip(1).all(|k| k.ai_controlled));
    }
}

#[test]
fn the_same_spec_and_seed_give_the_same_input_and_another_seed_gives_another() {
    let spec = ScaleSpec::new(20, 3);
    let a = generate(&spec, map()).unwrap();
    let b = generate(&spec, map()).unwrap();
    assert_eq!(a, b, "same spec, same input");
    assert_eq!(a.summary.input_digest, b.summary.input_digest);
    let mut other = spec.clone();
    other.seed += 1;
    let c = generate(&other, map()).unwrap();
    assert_ne!(a.summary.input_digest, c.summary.input_digest, "the seed changes the input");
    // 規模を変えても、先頭のステートは同じ配置になる(列挙順に依存しない決定的な生成)。
    let mut bigger = spec.clone();
    bigger.states = 21;
    let d = generate(&bigger, map()).unwrap();
    assert_eq!(a.layout.states, d.layout.states[..20].to_vec());
}

#[test]
fn generated_inputs_pass_the_existing_validation_and_the_shared_assembly() {
    let input = generate(&ScaleSpec::new(30, 3), map()).unwrap();
    // 世界: M0 のローダー(WorldModel::build)+地図との Authoring 検証。
    let world = build_world(map(), &input).unwrap();
    assert_eq!(world.states().len(), 30);
    assert_eq!(world.provinces().len(), 180);
    // 経済: 共有の組み立て(シナリオ・資源設定・所属の検証)。
    let sim = assemble(&input, &world, buildings()).unwrap();
    assert_eq!(sim.registry.len(), 30);
    assert_eq!(sim.countries.len(), 3);
    assert_eq!(sim.date, START);
    // 緩めていない: 国家に属さないステートを足せば共有の検証が拒否する。
    let mut broken = input.clone();
    broken.scenario.countries[0].states.pop();
    assert!(assemble(&broken, &world, buildings()).is_err());
    // 緩めていない: 地図に無い座標のプロヴィンスは世界の検証が拒否する。
    let mut bad_world = input.clone();
    bad_world.table.provinces[0].q = 99_999;
    assert!(build_world(map(), &bad_world).is_err());
}

#[test]
fn populations_and_buildings_exist_and_the_economy_actually_runs() {
    let input = generate(&ScaleSpec::new(12, 3), map()).unwrap();
    let world = build_world(map(), &input).unwrap();
    let mut sim = assemble(&input, &world, buildings()).unwrap();
    assert!(sim.registry.iter().all(|s| s.population >= 150_000));
    assert!(sim.registry.iter().all(|s| !s.buildings.is_empty()));
    assert!(input.summary.provinces_with_deposits > 0, "some deposits exist");
    let treasury_before: Vec<f64> = sim.countries.iter().map(|c| c.treasury).collect();
    let mut monthly = 0;
    for _ in 0..90 {
        let (_, rep) = sim.advance_day(buildings());
        monthly += rep.monthly_ran as u32;
    }
    assert!(monthly >= 2, "monthly economy ran ({monthly})");
    let treasury_after: Vec<f64> = sim.countries.iter().map(|c| c.treasury).collect();
    assert_ne!(treasury_before, treasury_after, "finance moved");
}

#[test]
fn the_manual_script_is_deterministic_and_targets_the_manual_country_states() {
    let input = generate(&ScaleSpec::new(3, 3), map()).unwrap();
    let a = Script::new(&input, true);
    let b = Script::new(&input, true);
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), 5, "1 state x 5 order days");
    assert!(Script::new(&input, false).is_empty());
}

// ---------------------------------------------------------------- 実行・決定性・保存再開

#[test]
fn repeated_runs_and_a_save_resume_agree_for_a_small_case() {
    let r = run(&small(6, 3, 100, 60));
    assert_eq!(r.status, CaseStatus::Success, "{}", r.render());
    // 連続3回+参照1回。すべて指定日数を処理し、同じ日付で終わる。
    assert_eq!(r.runs.len(), 4);
    assert!(r.runs.iter().all(|x| x.days == 100 && x.end_date == Some(date_after(100))));
    assert!(r.determinism_runs_equal);
    let c = r.continuity.as_ref().unwrap();
    assert_eq!(c.compared_days, 40);
    assert_eq!(c.first_date, Some(date_after(61)));
    assert_eq!(c.last_date, Some(date_after(100)));
    assert!(c.digest_mismatch_days.is_empty() && c.report_mismatch_days.is_empty());
    assert!(c.restored_equals_state_at_save && c.final_states_equal);
    assert!(c.file_bytes > 0 && c.queued_orders_at_save > 0 && c.monthly_days_in_compared > 0 && c.distinct_digests >= 2);
    assert!(r.summary.is_some());
}

#[test]
fn saving_and_resuming_agree_at_a_month_boundary_and_in_the_middle_of_a_month() {
    // 保存日の翌日が月替わりになる分割と、ならない分割を、実際の暦から選ぶ。
    let boundary = (50..70).find(|&k| date_after(k).next().1).expect("a month boundary exists");
    let middle = (50..70).find(|&k| !date_after(k).next().1).expect("a mid-month day exists");
    for (split, expect_boundary) in [(boundary, true), (middle, false)] {
        let r = run(&small(6, 3, 100, split));
        assert_eq!(r.status, CaseStatus::Success, "split {split}: {}", r.render());
        let c = r.continuity.as_ref().unwrap();
        assert_eq!(c.split_next_day_is_month_change, expect_boundary);
        assert!(c.queued_orders_at_save > 0, "construction was in progress at the save point");
        assert_eq!(c.compared_days, 100 - split);
    }
}

#[test]
fn a_wide_case_with_many_countries_also_agrees() {
    let r = run(&small(12, 6, 90, 45));
    assert_eq!(r.status, CaseStatus::Success, "{}", r.render());
    assert_eq!(r.summary.as_ref().unwrap().countries, 6);
}

#[test]
fn an_empty_or_unchanging_comparison_never_counts_as_success() {
    // 1月の最初の3日だけを比べても、月次処理が一度も走らないので成功にしない。
    let r = run(&small(3, 3, 3, 1));
    match &r.status {
        CaseStatus::CorrectnessFailure(v) => {
            assert!(v.iter().any(|m| m.contains("monthly")), "{v:?}");
        }
        other => panic!("expected a correctness failure, got {other:?}"),
    }
    // 後半が空になる指定は、実行前に不正として拒否する。
    assert!(small(3, 3, 10, 10).validate().is_err());
    assert!(small(3, 3, 10, 0).validate().is_err());
    assert!(small(3, 3, 10, 11).validate().is_err());
}

#[test]
fn a_time_limit_is_reported_as_timed_out_not_as_a_failure_or_a_completed_measurement() {
    let mut spec = small(30, 3, 90, 45);
    spec.time_limit = Duration::from_nanos(1);
    let r = run(&spec);
    assert!(matches!(r.status, CaseStatus::TimedOut { .. }), "{:?}", r.status);
    assert_eq!(r.status.exit_code(), 3);
    assert!(r.continuity.is_none(), "no comparison result is invented for an unfinished case");
    assert!(!r.status.is_success());
}

// ---------------------------------------------------------------- 不正な指定

#[test]
fn invalid_sizes_and_arguments_are_rejected_explicitly() {
    let ok = ScaleSpec::new(3, 3);
    assert!(ok.validate().is_ok());
    let cases: [fn(&mut ScaleSpec); 9] = [
        |s| s.states = 0,
        |s| s.states = 20_001,
        |s| s.countries = 0,
        |s| s.countries = 4, // 国家数 > ステート数
        |s| s.provinces_per_state = 0,
        |s| s.provinces_per_state = 65,
        |s| s.days = 0,
        |s| s.runs = 0,
        |s| s.time_limit = Duration::ZERO,
    ];
    for (i, f) in cases.iter().enumerate() {
        let mut s = ok.clone();
        f(&mut s);
        assert!(matches!(s.validate(), Err(ScaleError::InvalidSpec(_))), "case {i}");
        assert!(generate(&s, map()).is_err(), "generation refuses case {i}");
    }
    let a = |v: &[&str]| parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert!(matches!(a(&["--states", "30"]), Ok(Command::Case { .. })));
    assert!(matches!(a(&["--series", "--wide"]), Ok(Command::Series { wide: true, .. })));
    assert!(a(&[]).is_err(), "states is required");
    assert!(a(&["--states"]).is_err(), "missing value");
    assert!(a(&["--states", "abc"]).is_err());
    assert!(a(&["--states", "-1"]).is_err());
    assert!(a(&["--states", "0"]).is_err());
    assert!(a(&["--states", "3", "--countries", "9"]).is_err());
    assert!(a(&["--states", "3", "--split", "360"]).is_err(), "split must be below days");
    assert!(a(&["--states", "3", "--bogus"]).is_err());
    assert!(a(&["--series", "--states", "3"]).is_err());
    assert!(a(&["--wide", "--states", "3"]).is_err());
    assert!(a(&["--series", "--sizes", "3,x"]).is_err());
    assert!(a(&["--series", "--sizes", "3,0"]).is_err());
    assert!(matches!(a(&["--help"]), Ok(Command::Help)));
    // 既定値の確認(記録と一致させる)。
    match a(&["--states", "30"]).unwrap() {
        Command::Case { spec, .. } => {
            assert_eq!((spec.countries, spec.days, spec.split_day, spec.runs, spec.seed), (3, 360, 180, 3, 24));
            assert_eq!(spec.time_limit, Duration::from_secs(120));
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_map_that_cannot_supply_the_land_is_reported_not_silently_shrunk() {
    let mut s = ScaleSpec::new(20_000, 3);
    s.provinces_per_state = 64;
    match generate(&s, map()) {
        Err(ScaleError::InsufficientLand { needed_states, built_states, .. }) => {
            assert_eq!(needed_states, 20_000);
            assert!(built_states < needed_states);
        }
        other => panic!("expected InsufficientLand, got {:?}", other.map(|i| i.summary)),
    }
}

// ---------------------------------------------------------------- 保存先

fn listing(dir: &str) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.metadata().map(|m| m.len()).unwrap_or(0)))
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

#[test]
fn the_harness_never_writes_to_the_user_save_directory_and_cleans_up_its_own_file() {
    let before = listing("saves/map_economy");
    let base = fresh_temp_base("clean");
    let r = run_case(map(), buildings(), &small(6, 3, 100, 60), &base);
    assert_eq!(r.status, CaseStatus::Success, "{}", r.render());
    assert!(r.continuity.as_ref().unwrap().file_bytes > 0, "a file was really written during the case");
    assert!(
        !base.join("map_economy").join("p32_024_scale.mapeco.ron").exists(),
        "the measurement save is removed afterwards"
    );
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(&base);
    assert_eq!(before, listing("saves/map_economy"), "the user's save directory is untouched");
}

#[test]
fn a_save_base_inside_a_saves_directory_is_refused_before_anything_is_written() {
    let base: PathBuf = std::env::temp_dir().join("p32_024_refuse").join("saves").join("x");
    let r = run_case(map(), buildings(), &small(3, 3, 10, 5), &base);
    assert!(matches!(r.status, CaseStatus::Unrunnable(_)), "{:?}", r.status);
    assert!(!base.exists());
}
