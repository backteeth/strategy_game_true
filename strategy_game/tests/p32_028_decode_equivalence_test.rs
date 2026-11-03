//! P32-028: `map_economy_file::decode` の高速経路(型つきの完全解析を先に試す)と、従来経路(`decode_legacy`)の同等性検証。
//!
//! 新旧に**同じ入力**を渡し、成功/失敗、成功時の解析済みの内容、エラーの種類と内容(`FileError` の完全一致と `Debug` 表現)、
//! `kind`・版の拒否順を比べる。新しい期待値を新実装に合わせることはしない(差が出たら新実装の適用条件を見直す)。
//! 入力は、実際の保存(m4d・m4e、小さい合成ケース、更新前の地図の固定fixture)から、位置を変えた切断・置換・並べ替え・
//! 挿入などで決定的に作る(乱数は固定の種)。時間は合否条件にしない。保存先は一時ディレクトリだけ。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use strategy_game::building::data::BuildingRegistry;
use strategy_game::economy::map_economy_file::{
    DecodePhases, FileError, SaveLocation, decode, decode_legacy, decode_timed, inspect, load, save,
};
use strategy_game::economy::map_session::{EconomySession, GeographyData, SessionPaths, load_buildings, scenario_by_id};
use strategy_game::economy::map_state::static_identity::StaticInputs;
use strategy_game::economy::phase_timing::Timing;
use strategy_game::economy::save_timing_bench::build_saved_text;
use strategy_game::economy::scale_harness::{ScaleSpec, fresh_temp_base};
use strategy_game::hexmap::data::{HexMap, load_from_path};
use strategy_game::world::LocationFacilityRegistry;

// ---------------------------------------------------------------- 基になる保存文章

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session(id: &str) -> EconomySession {
    EconomySession::load_files(scenario_by_id(id).unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!("p32_028_{tag}_{}_{nanos}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn dir(&self) -> PathBuf {
        self.0.join("map_economy")
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 共有APIで保存した、保存ファイルの内容(m4d / m4e。45日進めた状態で、建設注文が進行中)。
fn session_text(id: &str) -> String {
    let mut s = session(id);
    for _ in 0..45 {
        s.advance_day();
    }
    let tmp = Tmp::new(id);
    let r = s.save(&geo().map, geo().model(), &tmp.loc(), "x").unwrap();
    fs::read_to_string(r.path).unwrap()
}

fn map() -> &'static HexMap {
    static M: OnceLock<HexMap> = OnceLock::new();
    M.get_or_init(|| load_from_path("assets/data/hex_atlas_map.json").unwrap())
}

fn buildings() -> &'static BuildingRegistry {
    static B: OnceLock<BuildingRegistry> = OnceLock::new();
    B.get_or_init(|| load_buildings("assets/data/buildings.ron").unwrap())
}

fn synthetic_text(states: u32, countries: u32) -> String {
    let mut scale = ScaleSpec::new(states, countries);
    scale.days = 90;
    scale.split_day = 45;
    let base = fresh_temp_base("p28_text");
    build_saved_text(map(), buildings(), &scale, &base).unwrap().text
}

fn small() -> &'static str {
    static T: OnceLock<String> = OnceLock::new();
    T.get_or_init(|| synthetic_text(12, 3))
}

// ---------------------------------------------------------------- 比較

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    /// 高速経路で成功(工程は `fast` の1つだけ)。
    OkFast,
    /// 従来経路で成功(高速経路が使われなかったのに成功した。起きないはず)。
    OkFallback,
    Malformed,
    NotThisKindOther,
    NotThisKindLegacyV1,
    Unsupported,
    OtherError,
}

/// 新旧を同じ入力で比べる。違いがあれば、入力の名前つきで失敗させる。経路と結果の種類を返す。
fn compare(label: &str, text: &str, seen: &mut BTreeMap<Outcome, usize>) -> Outcome {
    let legacy = decode_legacy(text);
    let new = decode(text);
    // 値の完全一致(`PartialEq`)。ただし NaN を含む成功の結果は、NaN != NaN のため `PartialEq` では常に不一致になるので、
    // そのときだけ `Debug` 表現(浮動小数の値を完全に表す)の一致で比べる。エラーは常に `PartialEq` で比べる。
    let debug_new = format!("{new:?}");
    let debug_legacy = format!("{legacy:?}");
    assert_eq!(debug_new, debug_legacy, "Debug representation differs for {label}");
    if !(new.is_ok() && debug_new.contains("NaN")) {
        assert_eq!(new, legacy, "decode differs from decode_legacy for {label}");
    }
    if let Err(e) = &new {
        assert_eq!(e.to_string(), legacy.as_ref().unwrap_err().to_string(), "message differs for {label}");
    }
    // 経路: 計時つきの呼出(同じ結果を返す)で、どの工程を通ったかを見る
    let mut tm = Timing::on();
    let phases = DecodePhases { fast: "fast", header: "header", body: "body" };
    let timed = decode_timed(text, &mut tm, phases);
    assert_eq!(format!("{timed:?}"), debug_legacy, "decode_timed differs for {label}");
    if !(timed.is_ok() && debug_legacy.contains("NaN")) {
        assert_eq!(timed, legacy, "decode_timed differs for {label}");
    }
    let names: Vec<&str> = tm.spans().iter().map(|s| s.name).collect();
    let fast_only = names == ["fast"];
    if fast_only {
        assert!(legacy.is_ok(), "the fast path was used for an input the legacy path rejects: {label}");
    } else {
        assert!(names.first() == Some(&"fast") && names.contains(&"header"), "fallback must run the legacy path: {label} {names:?}");
    }
    let o = match &new {
        Ok(_) if fast_only => Outcome::OkFast,
        Ok(_) => Outcome::OkFallback,
        Err(FileError::Malformed(_)) => Outcome::Malformed,
        Err(FileError::NotThisKind(strategy_game::economy::map_economy_file::DetectedKind::Other(_))) => Outcome::NotThisKindOther,
        Err(FileError::NotThisKind(strategy_game::economy::map_economy_file::DetectedKind::LegacyV1 { .. })) => Outcome::NotThisKindLegacyV1,
        Err(FileError::UnsupportedVersion { .. }) => Outcome::Unsupported,
        Err(_) => Outcome::OtherError,
    };
    *seen.entry(o).or_default() += 1;
    o
}

/// 比較した入力の数と結果の内訳を標準エラーへ出す(`--nocapture` で記録に残す。判定には使わない)。
fn report(name: &str, seen: &BTreeMap<Outcome, usize>) {
    let total: usize = seen.values().sum();
    eprintln!("EQUIV {name}: inputs={total} {seen:?}");
}

// ---------------------------------------------------------------- 入力の部品(正常な保存の構造を使う)

const FIELDS: [&str; 4] = ["kind", "format_version", "identity", "snapshot"];

/// 保存文章の最上位を、(前置き `(\n`, 各フィールドの区間, 後置き `)`)に分ける(整形出力の構造: 字下げ4の `名前:` で始まる行)。
fn segments(text: &str) -> (String, Vec<String>, String) {
    let starts: Vec<usize> = FIELDS.iter().map(|f| text.find(&format!("\n    {f}:")).expect("field") + 1).collect();
    let end = text.rfind("\n)").expect("end") + 1;
    let mut segs = Vec::new();
    for (i, s) in starts.iter().enumerate() {
        let e = starts.get(i + 1).copied().unwrap_or(end);
        segs.push(text[*s..e].to_string());
    }
    (text[..starts[0]].to_string(), segs, text[end..].to_string())
}

fn assemble(head: &str, segs: &[&str], tail: &str) -> String {
    format!("{head}{}{tail}", segs.concat())
}

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x.clone());
            out.push(p);
        }
    }
    out
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn char_boundary_floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------- テスト

#[test]
fn real_saves_decode_identically_and_take_the_fast_path() {
    let mut seen = BTreeMap::new();
    let mut texts: Vec<(String, String)> = vec![
        ("m4d".into(), session_text("m4d")),
        ("m4e".into(), session_text("m4e")),
        ("synthetic12x3".into(), small().to_string()),
        ("synthetic30x5".into(), synthetic_text(30, 5)),
    ];
    // 更新前の地図で作った保存(形式は現行。内容の判定は別の層)
    for f in ["old_map_m4d", "old_map_m4e"] {
        texts.push((f.into(), fs::read_to_string(format!("tests/fixtures/p32_025/{f}.mapeco.ron")).unwrap()));
    }
    for (name, t) in &texts {
        assert_eq!(compare(name, t, &mut seen), Outcome::OkFast, "{name}: a valid current save takes the fast path");
        // 解析済みの内容が一致し、再エンコードすると元の文章に戻る(内容が欠けていない)
        let d = decode(t).unwrap();
        assert!(d == decode_legacy(t).unwrap());
        assert!(!d.snapshot.states.is_empty());
    }
    report("real_saves_decode_identically_and_take_the_fast_path", &seen);
}

#[test]
fn reordered_top_level_fields_are_identical() {
    let (head, segs, tail) = segments(small());
    let mut seen = BTreeMap::new();
    let mut ok = 0;
    for (i, p) in permutations(&[0usize, 1, 2, 3]).into_iter().enumerate() {
        let parts: Vec<&str> = p.iter().map(|&k| segs[k].as_str()).collect();
        let t = assemble(&head, &parts, &tail);
        if compare(&format!("perm{i} {p:?}"), &t, &mut seen) == Outcome::OkFast {
            ok += 1;
        }
    }
    // 順序を変えても、現行の保存として読める(kind・版が後方にあるものを含む)
    assert_eq!(ok, 24);
    report("reordered_top_level_fields_are_identical", &seen);
}

#[test]
fn missing_duplicated_and_extra_fields_are_identical() {
    let (head, segs, tail) = segments(small());
    let mut seen = BTreeMap::new();
    // 欠落(1つ、2つ)
    for mask in 0u8..16 {
        let parts: Vec<&str> = (0..4).filter(|k| mask & (1 << k) != 0).map(|k| segs[k].as_str()).collect();
        compare(&format!("subset{mask:04b}"), &assemble(&head, &parts, &tail), &mut seen);
    }
    // 重複(隣り合う、末尾)
    for k in 0..4 {
        let mut parts: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
        parts.insert(k, segs[k].as_str());
        compare(&format!("dup_adjacent{k}"), &assemble(&head, &parts, &tail), &mut seen);
        let mut parts: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
        parts.push(segs[k].as_str());
        compare(&format!("dup_tail{k}"), &assemble(&head, &parts, &tail), &mut seen);
    }
    // 別の値の重複(kind・版が2回、値が異なる)
    let kind_other = segs[0].replace("map_economy_state", "other_kind");
    let ver_other = segs[1].replace("2", "3");
    for (a, b) in [(&segs[0], &kind_other), (&kind_other, &segs[0]), (&segs[1], &ver_other), (&ver_other, &segs[1])] {
        let mut parts: Vec<&str> = vec![a.as_str(), b.as_str()];
        parts.extend(segs.iter().skip(if a.starts_with("    kind") { 1 } else { 0 }).map(|s| s.as_str()));
        compare("dup_conflicting", &assemble(&head, &parts, &tail), &mut seen);
    }
    // 既存仕様で許可されない追加フィールド(最上位・入れ子)と、旧V1の `version`
    for extra in ["    extra: 1,\n", "    version: 1,\n", "    version: 2,\n", "    extra: [1, 2, 3],\n", "    extra: (a: 1.5, b: \"x\"),\n"] {
        for pos in [0usize, 2, 4] {
            let mut parts: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
            parts.insert(pos, extra);
            compare(&format!("extra {extra:?}@{pos}"), &assemble(&head, &parts, &tail), &mut seen);
        }
    }
    let nested = small().replacen("states: [", "extra_in_snapshot: 1,\n        states: [", 1);
    compare("extra field inside snapshot", &nested, &mut seen);
    let nested = small().replacen("scheme: ", "extra_in_identity: [1, 2],\n        scheme: ", 1);
    compare("extra field inside identity", &nested, &mut seen);
    // 旧V1の形(`kind` なし、`version` あり)と、`kind` も `version` も無い
    for t in ["(version: 1, data: 5)", "(version: 1)", "(version: 3, kind: 5)", "(foo: 1)", "(kind: \"x\")", "(kind: None, version: 1)"] {
        compare(t, t, &mut seen);
    }
    assert!(seen.contains_key(&Outcome::Malformed) && seen.contains_key(&Outcome::NotThisKindLegacyV1));
    report("missing_duplicated_and_extra_fields_are_identical", &seen);
}

#[test]
fn kind_and_version_values_are_identical_including_boundary_literals() {
    let (head, segs, tail) = segments(small());
    let mut seen = BTreeMap::new();
    let kinds = [
        "\"map_economy_state\"", "\"other\"", "\"\"", "\"map_economy_state \"", "5", "Some(\"map_economy_state\")", "None",
        "r\"map_economy_state\"", "r#\"map_economy_state\"#", "\"map_economy\\u{5f}state\"", "\"map_economy_state\\n\"", "'k'",
        "\"MAP_ECONOMY_STATE\"", "map_economy_state", "[\"map_economy_state\"]",
    ];
    let versions = [
        "2", "1", "0", "3", "99", "-1", "2.0", "2.5", "\"2\"", "0x2", "0b10", "0o2", "+2", "2_", "1_0", "4294967295", "4294967296",
        "4294967298", "02", "2u32", "Some(2)", "None", "true", "()", "1e0", "inf",
    ];
    let mut ok = 0;
    for k in kinds {
        for v in versions {
            let kseg = format!("    kind: {k},\n");
            let vseg = format!("    format_version: {v},\n");
            let t = assemble(&head, &[&kseg, &vseg, &segs[2], &segs[3]], &tail);
            if compare(&format!("kind={k} version={v}"), &t, &mut seen) == Outcome::OkFast {
                ok += 1;
            }
            // 後方に置く(本体の後ろ)
            let t = assemble(&head, &[&segs[2], &segs[3], &kseg, &vseg], &tail);
            compare(&format!("late kind={k} version={v}"), &t, &mut seen);
        }
    }
    assert!(ok >= 4, "raw/escaped spellings of the current kind and version are accepted: {ok}");
    for o in [Outcome::NotThisKindOther, Outcome::Unsupported, Outcome::Malformed, Outcome::OkFast] {
        assert!(seen.contains_key(&o), "{o:?} must be covered");
    }
    report("kind_and_version_values_are_identical_including_boundary_literals", &seen);
}

#[test]
fn comments_whitespace_extensions_and_struct_names_are_identical() {
    let (head, segs, tail) = segments(small());
    let t0 = small();
    let mut seen = BTreeMap::new();
    let mut cases: Vec<(String, String)> = Vec::new();
    // コメント・空白・改行
    cases.push(("line comment first".into(), format!("// c\n{t0}")));
    cases.push(("block comment first".into(), format!("/* c */{t0}")));
    cases.push(("nested block comment".into(), format!("/* a /* b */ c */{t0}")));
    cases.push(("unterminated block comment".into(), format!("/* a {t0}")));
    cases.push(("comments between fields".into(), assemble(&head, &segs.iter().map(|s| format!("// x\n{s}/* y */")).collect::<Vec<_>>().iter().map(|s| s.as_str()).collect::<Vec<_>>(), &tail)));
    cases.push(("crlf".into(), t0.replace('\n', "\r\n")));
    cases.push(("tabs".into(), t0.replace("    ", "\t")));
    cases.push(("blank lines".into(), t0.replace('\n', "\n\n")));
    cases.push(("leading/trailing ws".into(), format!(" \n\t{t0} \n\n")));
    cases.push(("no trailing comma".into(), t0.replacen("),\n)", ")\n)", 1)));
    cases.push(("space before colon".into(), t0.replacen("kind:", "kind :", 1)));
    cases.push(("comment before colon".into(), t0.replacen("kind:", "kind /* c */ :", 1)));
    cases.push(("bom".into(), format!("\u{feff}{t0}")));
    // 拡張構文
    for ext in [
        "#![enable(implicit_some)]", "#![enable(unwrap_newtypes)]", "#![enable(unwrap_variant_newtypes)]", "#![enable(explicit_struct_names)]",
        "#![enable(implicit_some, unwrap_newtypes)]", "#![enable(foo)]", "#![enable()]", "#![disable(implicit_some)]", "#![enable(implicit_some)] #![enable(unwrap_newtypes)]",
    ] {
        cases.push((format!("ext {ext}"), format!("{ext}\n{t0}")));
    }
    cases.push(("attribute after value".into(), format!("{t0}\n#![enable(implicit_some)]")));
    // 構造体の名前・タプル形
    for name in ["File", "MapEconomyFile", "map_economy_state", "r#raw", "1bad"] {
        cases.push((format!("struct name {name}"), t0.replacen('(', &format!("{name}("), 1)));
    }
    cases.push(("space before paren".into(), t0.replacen('(', " (", 1)));
    cases.push((
        "positional tuple form".into(),
        format!("(\"map_economy_state\", 2, {}, {})", segs[2].trim_start().trim_start_matches("identity:").trim().trim_end_matches(','), segs[3].trim_start().trim_start_matches("snapshot:").trim().trim_end_matches(',')),
    ));
    for (n, t) in &cases {
        compare(n, t, &mut seen);
    }
    assert!(seen.contains_key(&Outcome::OkFast) && seen.contains_key(&Outcome::Malformed), "{seen:?}");
    report("comments_whitespace_extensions_and_struct_names_are_identical", &seen);
}

#[test]
fn numbers_strings_and_nesting_at_the_boundaries_are_identical() {
    let t0 = small();
    let mut seen = BTreeMap::new();
    // 数値(浮動小数・整数)の境界
    let float_tokens = [
        "0.0", "-0.0", "1.", ".5", "inf", "-inf", "NaN", "1e39", "-1e39", "1e-46", "3.4028235e38", "3.4028236e38", "0x10", "1_000.5", "+1.5", "1.5e", "1.5f32", "100", "1e3", "\"1.5\"", "Some(1.5)", "()",
    ];
    let int_tokens = [
        "0", "1", "4294967295", "4294967296", "-1", "18446744073709551615", "18446744073709551616", "0xFF", "0b1", "1_0", "1.0", "1e3", "\"5\"", "None", "+5", "00012", "1u32",
    ];
    for tok in float_tokens {
        let t = replace_value(t0, "living_standard: ", tok);
        compare(&format!("living_standard={tok}"), &t, &mut seen);
        let t = replace_value(t0, "unrest: ", tok);
        compare(&format!("unrest={tok}"), &t, &mut seen);
    }
    for tok in int_tokens {
        let t = replace_value(t0, "population: ", tok);
        compare(&format!("population={tok}"), &t, &mut seen);
        let t = replace_value(t0, "state: ", tok);
        compare(&format!("state={tok}"), &t, &mut seen);
    }
    // 文字列: 空、エスケープ、不正なエスケープ、未終端、生文字列、非ASCII
    for tok in ["\"\"", "\"a\\nb\"", "\"\\u{1F600}\"", "\"\\q\"", "\"unterminated", "r\"raw\"", "r#\"ra\"w\"#", "\"日本語\"", "\"\\u{110000}\"", "'c'"] {
        let t = replace_value(t0, "scheme: ", tok);
        compare(&format!("scheme={tok}"), &t, &mut seen);
        let t = replace_value(t0, "key: ", tok);
        compare(&format!("key={tok}"), &t, &mut seen);
    }
    // 入れ子の深さ(再帰の上限付近。未知フィールドへ深い値を入れる)
    for depth in [1usize, 10, 100, 127, 128, 129, 200] {
        for (open, close) in [("(", ")"), ("[", "]")] {
            let deep = format!("{}1{}", open.repeat(depth), close.repeat(depth));
            let t = t0.replacen("snapshot:", &format!("deep: {deep},\n    snapshot:"), 1);
            compare(&format!("deep unknown field {open}x{depth}"), &t, &mut seen);
            // 正当な位置(`identity.items` 全体を包む)への余分な括弧
            let t = t0.replacen("snapshot: ", &format!("snapshot: {}", open.repeat(depth.min(60))), 1);
            compare(&format!("extra open {open}x{depth}"), &t, &mut seen);
        }
    }
    assert!(seen.contains_key(&Outcome::OkFast) && seen.contains_key(&Outcome::Malformed));
    report("numbers_strings_and_nesting_at_the_boundaries_are_identical", &seen);
}

/// `marker` の最初の出現の直後の値(`,` または改行まで)を `tok` に置き換える。入力群を作るための操作で、判定には使わない。
fn replace_value(t: &str, marker: &str, tok: &str) -> String {
    let i = t.find(marker).unwrap_or_else(|| panic!("marker {marker}")) + marker.len();
    let rest = &t[i..];
    let j = rest.find([',', '\n']).unwrap_or(rest.len());
    format!("{}{}{}", &t[..i], tok, &rest[j..])
}

#[test]
fn empty_truncated_and_trailing_garbage_inputs_are_identical() {
    let t0 = small();
    let mut seen = BTreeMap::new();
    for t in ["", " ", "\n", "(", ")", "()", "(,)", "(kind", "(kind:", "(kind: \"map_economy_state\"", "(kind: \"map_economy_state\",", "[", "5", "\"x\"", "Some(", "\0"] {
        compare(&format!("short {t:?}"), t, &mut seen);
    }
    // 切断: 先頭付近は全位置、その後は間隔をあけ、各フィールド境界の前後も
    let (head, segs, _) = segments(t0);
    let mut cuts: Vec<usize> = (0..400.min(t0.len())).collect();
    cuts.extend((400..t0.len()).step_by(61));
    let mut at = head.len();
    for s in &segs {
        for d in [-2i64, -1, 0, 1, 2] {
            cuts.push((at as i64 + d).max(0) as usize);
        }
        at += s.len();
    }
    cuts.extend([t0.len() - 3, t0.len() - 2, t0.len() - 1]);
    cuts.sort_unstable();
    cuts.dedup();
    for c in cuts {
        let c = char_boundary_floor(t0, c);
        compare(&format!("cut@{c}"), &t0[..c], &mut seen);
    }
    // 後半だけ壊れた(割合 50〜99%): 1文字を壊す・消す・挿入する
    for pct in [50usize, 60, 70, 80, 90, 95, 99] {
        let p = char_boundary_floor(t0, t0.len() * pct / 100);
        for rep in ["@", ")", "(", "\"", "]", ",", "/*", "#"] {
            let q = char_boundary_floor(t0, p + 1);
            compare(&format!("replace@{pct}% by {rep}"), &format!("{}{}{}", &t0[..p], rep, &t0[q..]), &mut seen);
            compare(&format!("insert@{pct}% {rep}"), &format!("{}{}{}", &t0[..p], rep, &t0[p..]), &mut seen);
        }
        compare(&format!("delete@{pct}%"), &format!("{}{}", &t0[..p], &t0[char_boundary_floor(t0, p + 1)..]), &mut seen);
    }
    // 正常データの後ろに続く内容
    for tail in [" 5", ")", "()", "\n// c", "/* c */", "x", "\0", " (kind: 1)", "\n\n\n", "\u{feff}", ",", "\n(", "\n#![enable(implicit_some)]"] {
        compare(&format!("trailing {tail:?}"), &format!("{t0}{tail}"), &mut seen);
        compare(&format!("two documents {tail:?}"), &format!("{t0}{t0}{tail}"), &mut seen);
    }
    for o in [Outcome::OkFast, Outcome::Malformed] {
        assert!(seen.contains_key(&o), "{o:?}");
    }
    report("empty_truncated_and_trailing_garbage_inputs_are_identical", &seen);
}

#[test]
fn inputs_with_several_problems_at_once_are_identical() {
    let t0 = small();
    let (head, segs, tail) = segments(t0);
    let mut seen = BTreeMap::new();
    // 「kind・版の問題」× 「本体・構文の問題」の組み合わせ
    let kind_versions: Vec<(String, String, String)> = vec![
        ("map_economy_state".into(), "2".into(), "ok".into()),
        ("other".into(), "2".into(), "bad kind".into()),
        ("map_economy_state".into(), "3".into(), "bad version".into()),
        ("map_economy_state".into(), "1".into(), "version 1".into()),
        ("other".into(), "99".into(), "bad both".into()),
    ];
    let p = char_boundary_floor(t0, t0.len() * 3 / 4);
    for (k, v, name) in &kind_versions {
        let kseg = format!("    kind: \"{k}\",\n");
        let vseg = format!("    format_version: {v},\n");
        let body_problems: Vec<(String, String)> = vec![
            ("intact".into(), format!("{}{}", segs[2], segs[3])),
            ("snapshot cut".into(), format!("{}{}", segs[2], &segs[3][..char_boundary_floor(&segs[3], segs[3].len() / 2)])),
            ("identity empty".into(), format!("    identity: (),\n{}", segs[3])),
            ("snapshot wrong type".into(), format!("{}    snapshot: 5,\n", segs[2])),
            ("snapshot missing".into(), segs[2].to_string()),
            ("unknown field".into(), format!("{}    extra: 1,\n{}", segs[2], segs[3])),
            ("garbage in the second half".into(), format!("{}{}", segs[2], segs[3].replacen("states", "st@tes", 1))),
        ];
        for (bn, body) in &body_problems {
            compare(&format!("{name} + {bn}"), &format!("{head}{kseg}{vseg}{body}{tail}"), &mut seen);
            compare(&format!("{name} + {bn} (late kind)"), &format!("{head}{body}{kseg}{vseg}{tail}"), &mut seen);
            compare(&format!("{name} + {bn} (no tail)"), &format!("{head}{kseg}{vseg}{body}"), &mut seen);
        }
        // 全体を後半で切る
        let full = format!("{head}{kseg}{vseg}{}{}{tail}", segs[2], segs[3]);
        let c = char_boundary_floor(&full, full.len() * 3 / 4);
        compare(&format!("{name} + cut at 75%"), &full[..c], &mut seen);
        let _ = p;
    }
    // 旧V1の形 + 壊れた本体、`kind` 欠落 + 重複 など
    for t in [
        "(version: 1, kind: 5, snapshot: (", "(version: 1, version: 2)", "(kind: \"map_economy_state\", kind: \"x\")", "(kind: \"x\", format_version: 2, snapshot: ",
        "(version: 1, format_version: \"x\")", "(format_version: 2)", "(format_version: 3, version: 1)",
    ] {
        compare(t, t, &mut seen);
    }
    for o in [Outcome::NotThisKindOther, Outcome::Unsupported, Outcome::Malformed, Outcome::OkFast] {
        assert!(seen.contains_key(&o), "{o:?}: {seen:?}");
    }
    report("inputs_with_several_problems_at_once_are_identical", &seen);
}

#[test]
fn deterministic_random_edits_of_valid_saves_are_identical() {
    let bases: Vec<(&str, String)> = vec![("small", small().to_string()), ("m4e", session_text("m4e"))];
    let tokens = ["None", "Some(", "inf", "NaN", "..", "r#\"x\"#", "'c'", "{}", "0x1F", "1e3", "-0.0", "#![enable(implicit_some)]", "/*", "//", "\"", "(", ")", "[", "]", ":", ",", "kind", "version"];
    let chars: Vec<char> = "()[]{},:\"'/*#!@0129aZ_-+. \n\t".chars().collect();
    let mut seen = BTreeMap::new();
    let mut rng = Rng(28);
    let mut n = 0usize;
    for (name, base) in &bases {
        let rounds = if *name == "small" { 2500 } else { 150 };
        for round in 0..rounds {
            let mut t = base.clone();
            // 1〜3 個の編集
            for _ in 0..(1 + rng.below(3)) {
                if t.is_empty() {
                    break;
                }
                let p = char_boundary_floor(&t, rng.below(t.len()));
                match rng.below(6) {
                    0 => {
                        let q = char_boundary_floor(&t, p + 1 + rng.below(20));
                        t.replace_range(p..q, "");
                    }
                    1 => {
                        let q = char_boundary_floor(&t, p + 1);
                        t.replace_range(p..q, &chars[rng.below(chars.len())].to_string());
                    }
                    2 => {
                        let q = char_boundary_floor(&t, p + 1 + rng.below(30));
                        let dup = t[p..q].to_string();
                        t.insert_str(p, &dup);
                    }
                    3 => t.insert_str(p, tokens[rng.below(tokens.len())]),
                    4 => t.truncate(p),
                    _ => {
                        let q = char_boundary_floor(&t, p + 1);
                        let r = char_boundary_floor(&t, q + 1);
                        if r > q {
                            let a = t[p..q].to_string();
                            let b = t[q..r].to_string();
                            t.replace_range(p..r, &format!("{b}{a}"));
                        }
                    }
                }
            }
            compare(&format!("{name} random #{round}"), &t, &mut seen);
            n += 1;
        }
    }
    assert!(n >= 2650);
    // 多様な結果が含まれ(自明な比較ではない)、起きないはずの結果は無い
    assert!(seen.get(&Outcome::Malformed).copied().unwrap_or(0) > 100, "{seen:?}");
    assert!(seen.get(&Outcome::OkFast).copied().unwrap_or(0) > 0, "{seen:?}");
    assert_eq!(seen.get(&Outcome::OkFallback).copied().unwrap_or(0), 0, "legacy-only success must not exist: {seen:?}");
    assert_eq!(seen.get(&Outcome::OtherError).copied().unwrap_or(0), 0);
    report("deterministic_random_edits_of_valid_saves_are_identical", &seen);
}

// ---------------------------------------------------------------- 保存・復元・確認の経路

fn parts<'a>(b: &'a BuildingRegistry, f: &'a Option<LocationFacilityRegistry>) -> StaticInputs<'a> {
    StaticInputs { map: &geo().map, world: geo().model(), buildings: b, facilities: f.as_ref() }
}

#[test]
fn save_load_inspect_and_listing_use_the_fast_path_for_valid_saves_and_fall_back_for_others() {
    let mut s = session("m4e");
    for _ in 0..45 {
        s.advance_day();
    }
    let tmp = Tmp::new("paths");
    let before = s.sim.clone();
    save(&s.sim, &parts(&s.buildings, &s.facilities), &tmp.loc(), "good").unwrap();
    // 保存の読み戻し・load・inspect・一覧が、同じ decode を通って成功する(結果は従来経路と同じ)
    let text = fs::read_to_string(tmp.dir().join("good.mapeco.ron")).unwrap();
    assert_eq!(decode(&text), decode_legacy(&text));
    let mut r = session("m4e");
    let rep = inspect(&r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "good").unwrap();
    assert!(rep.verdict.is_exact() && matches!(rep.restorable, Some(Ok(()))));
    load(&mut r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "good").unwrap();
    assert!(r.sim == before);
    let list = EconomySession::list_saves(&tmp.loc());
    assert_eq!(list.entries.len(), 1);
    // 計時つきで、正常な保存が高速経路(fast の1工程)を通ること、壊れたファイルが従来経路へ戻ることを確認する
    let mut t = Timing::on();
    let mut r2 = session("m4e");
    strategy_game::economy::map_economy_file::load_timed(&mut r2.sim, &parts(&r2.buildings, &r2.facilities), &tmp.loc(), "good", &mut t).unwrap();
    assert!(t.span("load.decode_fast").is_some());
    assert!(t.span("load.decode_header").is_none() && t.span("load.decode_body").is_none(), "no legacy stage on the fast path");
    let mut ts = Timing::on();
    save_timed_to(&s, &tmp, "good2", &mut ts);
    assert!(ts.span("save.verify_decode_fast").is_some() && ts.span("save.verify_decode_header").is_none());

    // 壊れた・別の kind・旧V1・非対応の版: 従来経路へ戻り、従来と同じ拒否。現在の状態もファイルも変わらない
    let bad: Vec<(&str, String)> = vec![
        ("truncated", text[..text.len() / 2].to_string()),
        ("badkind", text.replacen("map_economy_state", "other", 1)),
        ("v1", "(version: 1, data: 5)".to_string()),
        ("badver", text.replacen("format_version: 2", "format_version: 3", 1)),
    ];
    for (name, content) in &bad {
        fs::write(tmp.dir().join(format!("{name}.mapeco.ron")), content).unwrap();
        let mut sim = session("m4e");
        let snapshot = sim.sim.clone();
        let mut t = Timing::on();
        let e = strategy_game::economy::map_economy_file::load_timed(&mut sim.sim, &parts(&sim.buildings, &sim.facilities), &tmp.loc(), name, &mut t).unwrap_err();
        assert_eq!(Err::<(), FileError>(e), Err(decode_legacy(content).unwrap_err()), "{name}: same rejection as the legacy decode");
        assert!(t.span("load.decode_header").is_some(), "{name}: fell back to the legacy path");
        assert!(sim.sim == snapshot, "{name}: the current state is unchanged");
        assert_eq!(fs::read_to_string(tmp.dir().join(format!("{name}.mapeco.ron"))).unwrap(), *content, "{name}: the file is unchanged");
    }
}

fn save_timed_to(s: &EconomySession, tmp: &Tmp, name: &str, t: &mut Timing) {
    strategy_game::economy::map_economy_file::save_timed(&s.sim, &parts(&s.buildings, &s.facilities), &tmp.loc(), name, t).unwrap();
}

#[test]
fn saved_file_bytes_do_not_depend_on_which_decode_path_verified_them() {
    // 保存の読み戻し検証は decode(高速経路)を使うが、保存ファイルのバイト列(encode の出力)は変わらない:
    // 同じ状態を保存した文章は、decode → 再 encode で、元の文章と完全に一致する。
    use strategy_game::economy::map_economy_file::encode;
    for text in [small().to_string(), session_text("m4d"), session_text("m4e")] {
        let d = decode(&text).unwrap();
        assert_eq!(encode(&d.snapshot, &d.identity).unwrap(), text);
        let l = decode_legacy(&text).unwrap();
        assert!(d == l);
    }
}

#[test]
fn inspect_timed_reports_the_same_result_as_inspect_and_records_its_stages() {
    use strategy_game::economy::map_economy_file::inspect_timed;
    use strategy_game::economy::phase_timing::Outcome;
    let mut s = session("m4e");
    for _ in 0..45 {
        s.advance_day();
    }
    let tmp = Tmp::new("inspect_timed");
    save(&s.sim, &parts(&s.buildings, &s.facilities), &tmp.loc(), "good").unwrap();
    let r = session("m4e");
    let before = r.sim.clone();
    let plain = inspect(&r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "good").unwrap();
    let mut t = Timing::on();
    let timed = inspect_timed(&r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "good", &mut t).unwrap();
    assert_eq!(plain, timed, "same report with and without timing");
    assert_eq!(t.outcome(), Outcome::Success);
    assert!(t.span("load.decode_fast").is_some() && t.span("load.decode_header").is_none(), "inspect also takes the fast path");
    assert!(t.span("restore.validate").is_some(), "compatible saves are checked for restorability");
    assert!(r.sim == before, "inspect never changes the session");
    // 拒否される保存(更新前の地図)も、確認自体は成功し、従来経路でなく高速経路で読まれ、復元可能性の確認には進まない
    fs::write(tmp.dir().join("old.mapeco.ron"), fs::read("tests/fixtures/p32_025/old_map_m4e.mapeco.ron").unwrap()).unwrap();
    let mut t2 = Timing::on();
    let rep = inspect_timed(&r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "old", &mut t2).unwrap();
    assert!(!rep.verdict.is_compatible() && rep.restorable.is_none());
    assert!(t2.span("restore.validate").is_none());
    // 壊れたファイルはエラーとして記録され、従来経路へ戻る
    fs::write(tmp.dir().join("broken.mapeco.ron"), "(kind: \"map_economy_state\", format_version: 2, identity:").unwrap();
    let mut t3 = Timing::on();
    assert!(inspect_timed(&r.sim, &parts(&r.buildings, &r.facilities), &tmp.loc(), "broken", &mut t3).is_err());
    assert_eq!(t3.outcome(), Outcome::Error);
    assert!(t3.span("load.decode_header").is_some());
}
