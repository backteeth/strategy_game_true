//! P32-027: 計時の整合性(同じ呼出の工程 ≤ 全体 ≤ 外側の時間)と、ヘッダー解析の比較測定の部品(`economy::parse_probe`、
//! `save_timing_bench::build_saved_text`)の自動検証。小さい入力だけを使う。**時間の値が一定以下であることは合否条件にしない**
//! (確認するのは、区間の包含関係・対応・入力の妥当性・製品の `decode` との同等性)。保存先は一時ディレクトリだけ。

use std::sync::OnceLock;

use strategy_game::building::data::BuildingRegistry;
use strategy_game::economy::map_economy_file::{DecodePhases, decode, decode_legacy_timed, decode_timed};
use strategy_game::economy::map_session::load_buildings;
use strategy_game::economy::parse_probe::{
    DEEP_LEVELS, Shape, fixture, parse_header_replica, parse_ignored_any, process_memory, text_stats,
};
use strategy_game::economy::phase_timing::Timing;
use strategy_game::economy::save_timing_bench::{BenchSpec, build_saved_text, run_bench_case};
use strategy_game::economy::scale_harness::{CaseStatus, ScaleSpec, fresh_temp_base};
use strategy_game::hexmap::data::{HexMap, load_from_path};

fn map() -> &'static HexMap {
    static M: OnceLock<HexMap> = OnceLock::new();
    M.get_or_init(|| load_from_path("assets/data/hex_atlas_map.json").unwrap())
}

fn buildings() -> &'static BuildingRegistry {
    static B: OnceLock<BuildingRegistry> = OnceLock::new();
    B.get_or_init(|| load_buildings("assets/data/buildings.ron").unwrap())
}

fn small_scale() -> ScaleSpec {
    let mut s = ScaleSpec::new(12, 3);
    s.days = 90;
    s.split_day = 45;
    s
}

// ---------------------------------------------------------------- 計時の整合性(同じ呼出の包含関係)

#[test]
fn spans_are_inside_the_call_and_the_call_is_inside_the_outer_measurement() {
    let spec = BenchSpec { scale: small_scale(), timed: true, trials: 2 };
    let base = fresh_temp_base("p27_test");
    let r = run_bench_case(map(), buildings(), &spec, &base);
    let _ = std::fs::remove_dir(base.join("map_economy"));
    let _ = std::fs::remove_dir(&base);
    assert_eq!(r.status, CaseStatus::Success, "{:?}", r.status);
    for t in &r.trials {
        for rec in [&t.save, &t.restore] {
            let tm = rec.timing.as_ref().unwrap();
            // 単調時計の区間の包含: 互いに重ならない工程の合計 <= 全体(入口〜出口) <= 外側から計った時間
            assert!(tm.covered() <= tm.total(), "stages are inside the call");
            assert!(tm.spans().iter().all(|s| s.total <= tm.total()), "each stage is inside the call");
            assert!(tm.total().as_secs_f64() * 1000.0 <= rec.wall_ms + 1e-6, "the call is inside the outer measurement");
            // 呼出ごとに新しい計時で、試行をまたいで蓄積しない(各工程の呼出は1回)
            assert!(tm.spans().iter().all(|s| s.calls == 1));
        }
    }
    // 出力の整合性の行(同じ試行・同じ操作の値だけで作る)がすべて成り立つ
    let text = r.render();
    let checks: Vec<&str> = text.lines().filter(|l| l.starts_with("CHECK ")).collect();
    assert_eq!(checks.len(), 4, "2 trials x (save, restore)");
    for c in checks {
        assert!(c.contains("covered_le_total=true") && c.contains("total_le_wall=true") && c.contains("max_span_le_total=true"), "{c}");
    }
}

// ---------------------------------------------------------------- 比較測定の入力

#[test]
fn text_stats_counts_a_known_valid_text() {
    let t = "(a: 1, b: [ (x: \"p(a)r:en\"), [1, 2] ], c: ((())))";
    let s = text_stats(t);
    // 文字列の中の記号は数えない
    assert_eq!((s.parens, s.brackets, s.fields), (1 + 1 + 3, 2, 4));
    assert_eq!(s.max_depth, 4);
    assert_eq!(s.bytes, t.len());
}

#[test]
fn every_fixture_shape_is_valid_ron_that_keeps_kind_and_version() {
    for shape in Shape::ALL {
        let text = fixture(shape, 20_000);
        assert!(text.len() >= 20_000, "{}", shape.name());
        // 妥当なRON(値として読める)で、ヘッダーの解析結果は kind・版のまま
        assert!(ron::from_str::<ron::Value>(&text).is_ok(), "{} must be valid RON", shape.name());
        assert_eq!(parse_header_replica(&text), Ok((Some("map_economy_state".to_string()), Some(2))), "{}", shape.name());
        assert!(parse_ignored_any(&text).is_ok(), "{}", shape.name());
        // 量を変えても形は同じ(要素数が増える)
        let bigger = text_stats(&fixture(shape, 40_000));
        let smaller = text_stats(&text);
        assert!(bigger.bytes > smaller.bytes);
        assert!(bigger.parens + bigger.brackets + bigger.fields >= smaller.parens + smaller.brackets + smaller.fields);
    }
    // 入れ子の形は、深さを持つ
    assert!(text_stats(&fixture(Shape::DeepNest, 20_000)).max_depth >= DEEP_LEVELS);
    assert!(text_stats(&fixture(Shape::FlatInts, 20_000)).max_depth <= 3);
}

#[test]
fn the_header_replica_reads_the_same_kind_and_version_as_the_product_on_a_real_save() {
    let base = fresh_temp_base("p27_text");
    let saved = build_saved_text(map(), buildings(), &small_scale(), &base).unwrap();
    assert_eq!((saved.states, saved.countries), (12, 3));
    assert!(saved.queued_orders > 0);
    // 製品の decode で読める保存文章(ファイルの内容そのもの)
    let product = decode(&saved.text).expect("the product save decodes");
    assert!(!product.snapshot.states.is_empty());
    assert_eq!(parse_header_replica(&saved.text), Ok((Some("map_economy_state".to_string()), Some(2))));
    // 計時つきの decode は、同じ結果を返し、ヘッダー・本体の2工程を1回ずつ記録する
    let mut tm = Timing::on();
    let phases = DecodePhases { fast: "fast", header: "header", body: "body" };
    let timed = decode_timed(&saved.text, &mut tm, phases).unwrap();
    assert!(timed == product);
    // P32-028: 正常な保存は高速経路(型つきの完全解析の1工程)だけを通る。従来経路(ヘッダー・本体の2工程)は同じ結果を返す。
    assert_eq!(tm.spans().iter().map(|s| (s.name, s.calls)).collect::<Vec<_>>(), [("fast", 1)]);
    let mut legacy = Timing::on();
    let l = decode_legacy_timed(&saved.text, &mut legacy, ("header", "body")).unwrap();
    assert!(l == product);
    assert_eq!(legacy.spans().iter().map(|s| (s.name, s.calls)).collect::<Vec<_>>(), [("header", 1), ("body", 1)]);
    // 無効な計時でも同じ結果
    let off = decode_timed(&saved.text, &mut Timing::off(), phases).unwrap();
    assert!(off == product);
    // 作業用のファイルは残っていない
    assert!(!base.exists() || std::fs::read_dir(&base).map(|d| d.count()).unwrap_or(0) == 0);
}

#[test]
fn product_decode_still_rejects_the_same_bad_inputs() {
    use strategy_game::economy::map_economy_file::FileError;
    // 製品の解析方法・拒否理由は変えていない(代表: 壊れた文章、別の kind、旧V1の形、非対応の版)。
    assert!(matches!(decode("(not ron"), Err(FileError::Malformed(_))));
    assert!(matches!(decode("(kind: \"other\", format_version: 2)"), Err(FileError::NotThisKind(_))));
    assert!(matches!(decode("(version: 1)"), Err(FileError::NotThisKind(_))));
    assert!(matches!(decode("(kind: \"map_economy_state\", format_version: 99)"), Err(FileError::UnsupportedVersion { found: 99 })));
    assert!(matches!(decode("(kind: \"map_economy_state\", format_version: 2)"), Err(FileError::Malformed(_))));
}

#[test]
fn process_memory_is_either_unmeasured_or_a_plausible_reading() {
    if let Some(m) = process_memory() {
        assert!(m.working_set > 0 && m.private_bytes > 0);
        // ピークはプロセス開始からの最大値なので、現在値より小さくならない
        assert!(m.peak_working_set >= m.working_set / 2);
    }
}
