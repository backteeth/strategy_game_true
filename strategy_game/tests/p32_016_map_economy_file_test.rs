//! P32-016: 新経路用RONファイルの保存・読込と原子的な置換のテスト。
//! 生成するファイルはすべてOSの一時ディレクトリ(テストごとに一意)に置き、リポジトリの `saves/` には
//! 書かない。旧V1の負例は、実際のユーザーセーブではなく**一時ディレクトリ内のコピー**で行う
//! (実ファイルは読み取りのみで、バイト列が変わらないことを確認する)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use strategy_game::building::data::{BuildingDefinition, BuildingRegistry};
use strategy_game::economy::map_economy_file::{
    DetectedKind, FileError, SaveIo, SaveLocation, TempFile, decode, encode, load, read_snapshot, save,
    save_with_io,
};
use strategy_game::economy::map_state::snapshot::{MapEconomySimulation, RestoreIssue};
use strategy_game::economy::map_state::static_identity::{StaticIdentity, StaticInputs};
use strategy_game::economy::map_state::{
    DayReport, MapStateEconomyRegistry, SimDate, parse_multi_month_scenario,
};
use strategy_game::economy::province_resources::{derive_state_resources, load_province_resources};
use strategy_game::world::{WorldModel, load_province_table, load_state_layout};

const START: SimDate = SimDate { year: 1800, month: 1, day: 1 };
const P: &str = "assets/data/world/preview/";

fn buildings() -> BuildingRegistry {
    let defs: Vec<BuildingDefinition> =
        ron::from_str(&fs::read_to_string("assets/data/buildings.ron").unwrap()).unwrap();
    BuildingRegistry { definitions: defs.into_iter().map(|d| (d.building_type, d)).collect() }
}

fn world() -> WorldModel {
    WorldModel::build(
        load_province_table(&format!("{P}provinces.ron")).unwrap(),
        load_state_layout(&format!("{P}states.ron")).unwrap(),
    )
    .unwrap()
}

fn make(name: &str, w: &WorldModel, b: &BuildingRegistry) -> MapEconomySimulation {
    let sc = parse_multi_month_scenario(&fs::read_to_string(format!("{P}{name}")).unwrap()).unwrap();
    let (mut reg, cs) = MapStateEconomyRegistry::from_multi_scenario(&sc, w, b).unwrap();
    let f = load_province_resources(&format!("{P}province_resources_m4d.ron")).unwrap();
    let d = derive_state_resources(w, &f).unwrap();
    reg.attach_province_resources(&d, w, &f).unwrap();
    MapEconomySimulation::new(START, reg, cs)
}

fn map() -> &'static strategy_game::hexmap::data::HexMap {
    static MAP: std::sync::OnceLock<strategy_game::hexmap::data::HexMap> = std::sync::OnceLock::new();
    MAP.get_or_init(|| strategy_game::hexmap::data::load_from_path("assets/data/hex_atlas_map.json").unwrap())
}

fn inp<'a>(w: &'a WorldModel, b: &'a BuildingRegistry) -> StaticInputs<'a> {
    StaticInputs { map: map(), world: w, buildings: b, facilities: None }
}

/// シミュレーションの現在の状態を、識別情報つきで封筒に入れた文字列にする。
fn enc(sim: &MapEconomySimulation, w: &WorldModel, b: &BuildingRegistry) -> Result<String, FileError> {
    let id = StaticIdentity::compute(sim, &inp(w, b)).unwrap().to_saved();
    encode(&sim.snapshot(w, b), &id)
}

fn m4d(w: &WorldModel, b: &BuildingRegistry) -> MapEconomySimulation {
    make("economy_scenario_m4d.ron", w, b)
}

fn advanced(w: &WorldModel, b: &BuildingRegistry, days: usize) -> MapEconomySimulation {
    let mut s = m4d(w, b);
    for _ in 0..days {
        s.advance_day(b);
    }
    s
}

/// 一意な一時ディレクトリ(Dropで削除。自分が作ったものだけ)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!(
            "p32_016_{tag}_{}_{nanos}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join("map_economy").join(format!("{name}.mapeco.ron"))
    }
    fn names(&self) -> Vec<String> {
        let dir = self.0.join("map_economy");
        let mut v: Vec<String> = fs::read_dir(dir)
            .map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
    fn write_raw(&self, name: &str, content: &[u8]) {
        fs::create_dir_all(self.0.join("map_economy")).unwrap();
        fs::write(self.file(name), content).unwrap();
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const V1_PATH: &str = "saves/savegame_v1.ron";

// ---------------------------------------------------------------- 形式

#[test]
fn a_saved_file_round_trips_into_an_independent_simulation() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("roundtrip");
    let sim = advanced(&w, &b, 100);
    let r = save(&sim, &inp(&w, &b), &t.loc(), "slot-1").unwrap();
    assert_eq!(r.path, t.file("slot-1"));
    assert!(!r.replaced_existing);
    assert_eq!((r.date.year, r.date.month, r.date.day), (1800, 4, 11));
    let mut target = m4d(&w, &b);
    let l = load(&mut target, &inp(&w, &b), &t.loc(), "slot-1").unwrap();
    assert_eq!(l.date, r.date);
    assert_eq!(target, sim);
    assert_eq!(t.names(), vec!["slot-1.mapeco.ron".to_string()], "no temporary file remains");
    // 保存したテキストは決定的で、再保存しても同一。
    let first = fs::read(t.file("slot-1")).unwrap();
    save(&sim, &inp(&w, &b), &t.loc(), "slot-1").unwrap();
    assert_eq!(first, fs::read(t.file("slot-1")).unwrap());
}

fn tamper(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "test setup: {from:?} not found");
    text.replacen(from, to, 1)
}

fn remove_line_containing(text: &str, needle: &str) -> String {
    let mut removed = false;
    let out: Vec<&str> = text
        .lines()
        .filter(|l| {
            if !removed && l.contains(needle) {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect();
    assert!(removed, "test setup: line with {needle:?} not found");
    out.join("\n")
}

/// 壊れた/不正なファイルは、読み込みが拒否され、現在の状態(日付・国庫・キュー・履歴)は変わらない。
fn assert_load_rejected(content: &[u8], check: impl Fn(&FileError) -> bool) {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("reject");
    t.write_raw("bad", content);
    let mut target = advanced(&w, &b, 40);
    let before = target.clone();
    let e = load(&mut target, &inp(&w, &b), &t.loc(), "bad").expect_err("must be rejected");
    assert!(check(&e), "unexpected error: {e}");
    assert_eq!(target, before, "a failed load must not change the current state");
}

#[test]
fn other_kinds_unsupported_versions_and_broken_files_are_rejected_without_changing_the_state() {
    let (w, b) = (world(), buildings());
    let good = enc(&advanced(&w, &b, 100), &w, &b).unwrap();
    decode(&good).unwrap();

    assert_load_rejected(tamper(&good, "kind: \"map_economy_state\"", "kind: \"other_kind\"").as_bytes(), |e| {
        matches!(e, FileError::NotThisKind(DetectedKind::Other(k)) if k == "other_kind")
    });
    assert_load_rejected(tamper(&good, "format_version: 2", "format_version: 3").as_bytes(), |e| {
        matches!(e, FileError::UnsupportedVersion { found: 3 })
    });
    assert_load_rejected(tamper(&good, "format_version: 2", "format_version: 0").as_bytes(), |e| {
        matches!(e, FileError::UnsupportedVersion { found: 0 })
    });
    assert_load_rejected(remove_line_containing(&good, "format_version").as_bytes(), |e| matches!(e, FileError::Malformed(_)));
    assert_load_rejected(remove_line_containing(&good, "kind:").as_bytes(), |e| matches!(e, FileError::Malformed(_)));
    assert_load_rejected(tamper(&good, "    kind:", "    extra: 1,\n    kind:").as_bytes(), |e| matches!(e, FileError::Malformed(_)));
    assert_load_rejected(b"", |e| matches!(e, FileError::Malformed(_)));
    assert_load_rejected(&[0xff, 0xfe, 0x00, 0x01], |e| matches!(e, FileError::Malformed(_)));
    assert_load_rejected(b"not ron at all", |e| matches!(e, FileError::Malformed(_)));
    // 途中切れ: どこで切れても読み込めない(部分的な状態を採用しない)。
    for tenth in 1..10 {
        let cut = good.len() * tenth / 10;
        assert_load_rejected(&good.as_bytes()[..cut], |e| matches!(e, FileError::Malformed(_)));
    }
    // 値の破損(非有限の国庫)は、構文として読めても復元の検証で拒否される。
    assert_load_rejected(tamper(&good, "treasury:", "treasury: NaN, x:").as_bytes(), |_| true);
    let nan = good.lines().find(|l| l.trim_start().starts_with("treasury:")).unwrap().to_string();
    assert_load_rejected(good.replacen(&nan, "            treasury: NaN,", 1).as_bytes(), |e| {
        matches!(e, FileError::Restore(r) if r.issues.iter().any(|i| matches!(i, RestoreIssue::InvalidValue(_))))
            || matches!(e, FileError::Malformed(_))
    });
}

#[test]
fn a_missing_required_field_is_rejected_but_an_explicitly_empty_one_is_accepted() {
    let (w, b) = (world(), buildings());
    // 日0: 履歴は明示的に空(`[]`)、注文一覧も空。これは正当で読み込める。
    let fresh = m4d(&w, &b);
    let text0 = enc(&fresh, &w, &b).unwrap();
    assert!(text0.contains("supply: []") && text0.contains("construction_queue: []") && text0.contains("order_log: []"));
    let t = Tmp::new("empty");
    t.write_raw("empty", text0.as_bytes());
    let mut target = advanced(&w, &b, 10);
    load(&mut target, &inp(&w, &b), &t.loc(), "empty").unwrap();
    assert_eq!(target, fresh);
    // 同じ欄が欠落したファイルは、既定値(空)で補完せず拒否する。
    for needle in ["supply: []", "iron: []", "factory: []", "construction_queue: []", "order_log: []", "next_order_seq", "dirty:", "ai: Some", "context:", "date:", "states:", "countries:", "stockpile:", "treasury:", "paid_cost"] {
        let text = if needle == "paid_cost" { enc(&advanced(&w, &b, 100), &w, &b).unwrap() } else { text0.clone() };
        if !text.contains(needle) {
            continue; // 日0に注文が無いなど、この欄が存在しない場合は別の確認(下)で扱う
        }
        let bad = remove_line_containing(&text, needle);
        assert_load_rejected(bad.as_bytes(), |e| matches!(e, FileError::Malformed(_)));
    }
    // 実行中の履歴・注文を持つファイルの必須項目(標本の `units`、注文の各欄)の欠落。
    let mut busy = advanced(&w, &b, 100);
    // 注文を持つ状態にする(AIが出した注文が進行中)。
    for _ in 0..30 {
        busy.advance_day(&b);
        if busy.countries.iter().any(|c| !c.construction_queue.is_empty()) {
            break;
        }
    }
    let text = enc(&busy, &w, &b).unwrap();
    for needle in ["units:", "unmet_milli:", "signature:", "mine_levels:", "last_monthly_evaluation_day"] {
        let bad = remove_line_containing(&text, needle);
        assert_load_rejected(bad.as_bytes(), |e| matches!(e, FileError::Malformed(_)));
    }
    if busy.countries.iter().any(|c| !c.construction_queue.is_empty()) {
        for needle in ["paid_cost:", "required_progress:", "target_level:", "progress:"] {
            let bad = remove_line_containing(&text, needle);
            assert_load_rejected(bad.as_bytes(), |e| matches!(e, FileError::Malformed(_)));
        }
    }
}

#[test]
fn the_context_in_the_file_is_checked_against_the_target_not_adopted() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("context");
    let sim = advanced(&w, &b, 60);
    save(&sim, &inp(&w, &b), &t.loc(), "m4d").unwrap();
    // 別のシナリオ(同じMapStateIdを含む)の集約へは復元できない。
    let mut other = make("economy_scenario_m4e.ron", &w, &b);
    let before = other.clone();
    let e = load(&mut other, &inp(&w, &b), &t.loc(), "m4d").unwrap_err();
    // P32-017: 項目別の判定(Incompatible)で拒否される。判定を通る差でも、内部コンテキスト照合が拒否する。
    assert!(
        matches!(&e, FileError::Incompatible(_))
            || matches!(&e, FileError::Restore(r) if r.issues.contains(&RestoreIssue::ContextMismatch)),
        "{e}"
    );
    assert_eq!(other, before);
    // ファイルの `context` を書き換えても(別の値を「正しい」とは採用せず)、復元先の値と一致しなければ拒否。
    let text = fs::read_to_string(t.file("m4d")).unwrap();
    let line = text.lines().find(|l| l.trim_start().starts_with("context:")).unwrap();
    t.write_raw("m4d", text.replacen(line, "        context: 1,", 1).as_bytes());
    let mut target = m4d(&w, &b);
    let e = load(&mut target, &inp(&w, &b), &t.loc(), "m4d").unwrap_err();
    assert!(matches!(&e, FileError::Restore(r) if r.issues.contains(&RestoreIssue::ContextMismatch)), "{e}");
}

// ---------------------------------------------------------------- 旧V1との区別

#[test]
fn the_legacy_v1_save_is_neither_loaded_nor_overwritten_and_stays_byte_identical() {
    let (w, b) = (world(), buildings());
    let real_before = fs::read(V1_PATH).unwrap();
    let t = Tmp::new("v1");
    // 一時ディレクトリ内のコピーだけを使う(`saves/` へは書かない)。
    fs::create_dir_all(t.0.join("map_economy")).unwrap();
    fs::copy(V1_PATH, t.0.join("savegame_v1.ron")).unwrap();
    fs::copy(V1_PATH, t.file("v1")).unwrap();
    let copy_before = fs::read(t.0.join("savegame_v1.ron")).unwrap();
    let in_dir_before = fs::read(t.file("v1")).unwrap();

    // V1を新形式として復元しない。
    let mut target = advanced(&w, &b, 20);
    let before = target.clone();
    let e = load(&mut target, &inp(&w, &b), &t.loc(), "v1").unwrap_err();
    assert!(matches!(e, FileError::NotThisKind(DetectedKind::LegacyV1 { version: 1 })), "{e}");
    assert!(e.to_string().contains("legacy V1"));
    assert_eq!(target, before);
    // V1を自動で上書きしない。
    let e = save(&target, &inp(&w, &b), &t.loc(), "v1").unwrap_err();
    assert!(matches!(e, FileError::OverwriteRefused { .. }), "{e}");
    assert_eq!(fs::read(t.file("v1")).unwrap(), in_dir_before);
    // 名前 `savegame_v1` でも、保存先は `savegame_v1.mapeco.ron`(V1のスロットとは別のファイル)。
    save(&target, &inp(&w, &b), &t.loc(), "savegame_v1").unwrap();
    assert_eq!(fs::read(t.0.join("savegame_v1.ron")).unwrap(), copy_before);
    // 新形式のファイルを、V1として読めない(旧V1のDTOとして解析できない)。
    let text = fs::read_to_string(t.file("savegame_v1")).unwrap();
    assert!(ron::from_str::<strategy_game::save::SaveGameV1>(&text).is_err());
    // 実際のV1ファイルは変更されていない。
    assert_eq!(fs::read(V1_PATH).unwrap(), real_before);
}

// ---------------------------------------------------------------- 保存先の制限

#[test]
fn names_and_locations_cannot_escape_the_allowed_directory() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("escape");
    let sim = advanced(&w, &b, 5);
    let long = "a".repeat(65);
    for name in [
        "", "..", "../x", "a/b", "a\\b", "C:\\x", "C:x", "/abs", "\\\\server\\share", "x.y", "x.mapeco.ron", ".hidden", "con", "NUL", "com1",
        "LPT9", "a b", "é", "a\0b", "../savegame_v1", "..\\saves\\savegame_v1", long.as_str(),
    ] {
        let e = save(&sim, &inp(&w, &b), &t.loc(), name).unwrap_err();
        assert!(matches!(e, FileError::InvalidName(_)), "{name:?}: {e}");
        let mut target = sim.clone();
        assert!(matches!(load(&mut target, &inp(&w, &b), &t.loc(), name).unwrap_err(), FileError::InvalidName(_)), "{name:?}");
    }
    assert!(!t.0.join("map_economy").exists(), "nothing is created for refused names");
    assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
}

fn make_dir_link(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        // ジャンクション(管理者権限不要)。
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

fn make_file_symlink(link: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
}

#[test]
fn a_directory_link_that_points_elsewhere_is_refused() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("dirlink");
    let outside = Tmp::new("outside");
    let sim = advanced(&w, &b, 5);
    if !make_dir_link(&t.0.join("map_economy"), &outside.0) {
        eprintln!("SKIPPED: directory links cannot be created in this environment");
        return;
    }
    let e = save(&sim, &inp(&w, &b), &t.loc(), "x").unwrap_err();
    assert!(matches!(e, FileError::Symlink(_) | FileError::LocationRefused(_)), "{e}");
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0, "nothing is written through the link");
    let mut target = sim.clone();
    assert!(load(&mut target, &inp(&w, &b), &t.loc(), "x").is_err());
    // ジャンクションの削除(リンク先は消さない)。
    let _ = fs::remove_dir(t.0.join("map_economy"));
}

#[test]
fn a_file_symlink_pointing_at_the_legacy_save_is_refused_and_the_target_is_untouched() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("filelink");
    fs::create_dir_all(t.0.join("map_economy")).unwrap();
    fs::copy(V1_PATH, t.0.join("savegame_v1.ron")).unwrap();
    let v1_before = fs::read(t.0.join("savegame_v1.ron")).unwrap();
    if !make_file_symlink(&t.file("slot"), &t.0.join("savegame_v1.ron")) {
        eprintln!("SKIPPED: file symlinks cannot be created in this environment (privilege)");
        return;
    }
    let sim = advanced(&w, &b, 5);
    let e = save(&sim, &inp(&w, &b), &t.loc(), "slot").unwrap_err();
    assert!(matches!(e, FileError::Symlink(_)), "{e}");
    let mut target = sim.clone();
    assert!(matches!(load(&mut target, &inp(&w, &b), &t.loc(), "slot").unwrap_err(), FileError::Symlink(_)));
    assert_eq!(fs::read(t.0.join("savegame_v1.ron")).unwrap(), v1_before);
}

// ---------------------------------------------------------------- 上書きの規則

#[test]
fn only_a_valid_map_economy_file_is_overwritten() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("overwrite");
    let old = advanced(&w, &b, 40);
    let new = advanced(&w, &b, 90);
    save(&old, &inp(&w, &b), &t.loc(), "slot").unwrap();
    let r = save(&new, &inp(&w, &b), &t.loc(), "slot").unwrap();
    assert!(r.replaced_existing);
    let mut target = m4d(&w, &b);
    load(&mut target, &inp(&w, &b), &t.loc(), "slot").unwrap();
    assert_eq!(target, new, "the file now holds the new state completely");
    assert_eq!(t.names(), vec!["slot.mapeco.ron".to_string()]);

    let good = enc(&old, &w, &b).unwrap();
    let refused: Vec<(&str, Vec<u8>)> = vec![
        ("other kind", tamper(&good, "map_economy_state", "something_else").into_bytes()),
        ("unsupported version", tamper(&good, "format_version: 2", "format_version: 9").into_bytes()),
        ("garbage", b"hello".to_vec()),
        ("empty", Vec::new()),
        ("truncated own file", good.as_bytes()[..good.len() / 2].to_vec()),
        ("legacy V1", fs::read(V1_PATH).unwrap()),
    ];
    for (what, content) in refused {
        t.write_raw("slot", &content);
        let e = save(&new, &inp(&w, &b), &t.loc(), "slot").unwrap_err();
        assert!(matches!(e, FileError::OverwriteRefused { .. }), "{what}: {e}");
        assert_eq!(fs::read(t.file("slot")).unwrap(), content, "{what}: the existing file is untouched");
        assert_eq!(t.names(), vec!["slot.mapeco.ron".to_string()], "{what}: no stray file");
    }
    // 通常ファイルでないもの(ディレクトリ)。
    fs::remove_file(t.file("slot")).unwrap();
    fs::create_dir(t.file("slot")).unwrap();
    assert!(save(&new, &inp(&w, &b), &t.loc(), "slot").is_err());
    assert!(t.file("slot").is_dir());
}

// ---------------------------------------------------------------- 原子的な置換と失敗の再現

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fail {
    None,
    Create,
    PartialWrite,
    Flush,
    Sync,
    Replace,
    ReplaceAndCleanup,
}

struct FakeIo(Fail);
struct FakeTemp {
    inner: Box<dyn TempFile>,
    fail: Fail,
}

fn injected(what: &str) -> io::Error {
    io::Error::other(format!("injected {what} failure"))
}

impl TempFile for FakeTemp {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.fail == Fail::PartialWrite {
            self.inner.write_all(&bytes[..bytes.len() / 2])?;
            return Err(injected("write"));
        }
        self.inner.write_all(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail == Fail::Flush {
            return Err(injected("flush"));
        }
        self.inner.flush()
    }
    fn sync(&mut self) -> io::Result<()> {
        if self.fail == Fail::Sync {
            return Err(injected("sync"));
        }
        self.inner.sync()
    }
}

impl SaveIo for FakeIo {
    fn create_temp(&self, path: &Path) -> io::Result<Box<dyn TempFile>> {
        if self.0 == Fail::Create {
            return Err(injected("create"));
        }
        let inner = strategy_game::economy::map_economy_file::RealIo.create_temp(path)?;
        Ok(Box::new(FakeTemp { inner, fail: self.0 }))
    }
    fn replace(&self, temp: &Path, final_path: &Path) -> io::Result<()> {
        if matches!(self.0, Fail::Replace | Fail::ReplaceAndCleanup) {
            return Err(injected("replace"));
        }
        strategy_game::economy::map_economy_file::RealIo.replace(temp, final_path)
    }
    fn remove_temp(&self, path: &Path) -> io::Result<()> {
        if self.0 == Fail::ReplaceAndCleanup {
            return Err(injected("cleanup"));
        }
        strategy_game::economy::map_economy_file::RealIo.remove_temp(path)
    }
}

#[test]
fn a_failure_while_creating_writing_flushing_syncing_or_replacing_keeps_the_existing_file_intact() {
    let (w, b) = (world(), buildings());
    let old = advanced(&w, &b, 40);
    let new = advanced(&w, &b, 90);
    for fail in [Fail::Create, Fail::PartialWrite, Fail::Flush, Fail::Sync, Fail::Replace] {
        let t = Tmp::new("fail");
        save(&old, &inp(&w, &b), &t.loc(), "slot").unwrap();
        // 無関係なファイル(古い一時ファイル風の名前を含む)は、失敗時の後始末で触られない。
        t.write_raw("other", b"unrelated");
        fs::write(t.0.join("map_economy").join("keep.txt"), b"keep").unwrap();
        fs::write(t.0.join("map_economy").join("slot.mapeco.ron.1-2-3.tmp"), b"stale").unwrap();
        let bytes_before = fs::read(t.file("slot")).unwrap();
        let names_before = t.names();
        let live_before = new.clone();
        let e = save_with_io(&new, &inp(&w, &b), &t.loc(), "slot", &FakeIo(fail)).unwrap_err();
        assert!(matches!(e, FileError::Io { .. }), "{fail:?}: {e}");
        assert!(e.to_string().contains("injected"), "{fail:?}: {e}");
        assert_eq!(fs::read(t.file("slot")).unwrap(), bytes_before, "{fail:?}: the existing file is byte-identical");
        assert_eq!(t.names(), names_before, "{fail:?}: our temporary file is removed; nothing else changed");
        assert_eq!(fs::read(t.0.join("map_economy").join("keep.txt")).unwrap(), b"keep");
        assert_eq!(live_before, new, "the running simulation is untouched");
        let mut target = m4d(&w, &b);
        load(&mut target, &inp(&w, &b), &t.loc(), "slot").unwrap();
        assert_eq!(target, old, "{fail:?}: the old state is still completely loadable");
    }
}

#[test]
fn a_failed_first_save_leaves_no_save_file_and_a_failed_cleanup_is_reported() {
    let (w, b) = (world(), buildings());
    let sim = advanced(&w, &b, 40);
    for fail in [Fail::Create, Fail::PartialWrite, Fail::Flush, Fail::Sync, Fail::Replace] {
        let t = Tmp::new("firstfail");
        assert!(save_with_io(&sim, &inp(&w, &b), &t.loc(), "slot", &FakeIo(fail)).is_err());
        assert!(!t.file("slot").exists(), "{fail:?}: no final file is created by a failed save");
        assert!(t.names().is_empty(), "{fail:?}: {:?}", t.names());
    }
    // 置換の失敗後に一時ファイルの削除も失敗した場合: 保存は失敗として報告され、その旨が添えられる。
    let t = Tmp::new("cleanupfail");
    let e = save_with_io(&sim, &inp(&w, &b), &t.loc(), "slot", &FakeIo(Fail::ReplaceAndCleanup)).unwrap_err();
    assert!(e.to_string().contains("could not be removed"), "{e}");
    assert!(!t.file("slot").exists());
    assert_eq!(t.names().len(), 1, "only our own leftover temporary file remains");
    assert!(t.names()[0].ends_with(".tmp"));
    // 通常のIOを通した保存(`Fail::None`)は成功し、完全な新しいファイルになる。
    let r = save_with_io(&sim, &inp(&w, &b), &t.loc(), "slot", &FakeIo(Fail::None)).unwrap();
    assert!(r.path.exists());
    let mut target = m4d(&w, &b);
    load(&mut target, &inp(&w, &b), &t.loc(), "slot").unwrap();
    assert_eq!(target, sim);
}

// ---------------------------------------------------------------- 復元と再開(同一プロセス内)

#[test]
fn resuming_from_a_file_matches_the_continuous_run_at_key_boundaries() {
    let (w, b) = (world(), buildings());
    // 連続実行の記録。
    let mut cont = m4d(&w, &b);
    let mut reports: Vec<(SimDate, DayReport)> = Vec::new();
    let mut snaps = vec![cont.snapshot(&w, &b)];
    for _ in 0..300 {
        reports.push(cont.advance_day(&b));
        snaps.push(cont.snapshot(&w, &b));
    }
    let monthly: Vec<usize> = (0..300).filter(|d| reports[*d].1.monthly_ran).collect();
    let done = (0..300).find(|d| !reports[*d].1.completed.is_empty()).expect("a completion exists");
    // 月末の前後、建設完成の直前・直後、履歴(4か月の窓)が揃う前後、AI初回評価の前。
    let mut points = vec![0, monthly[0], monthly[0] + 1, monthly[3] - 1, monthly[3], monthly[4], done, done + 1, 150, 299];
    points.sort_unstable();
    points.dedup();
    let t = Tmp::new("resume");
    for k in points {
        let sim = advanced(&w, &b, k);
        assert_eq!(sim.snapshot(&w, &b), snaps[k], "setup at day {k}");
        save(&sim, &inp(&w, &b), &t.loc(), "resume").unwrap();
        let mut r = m4d(&w, &b);
        load(&mut r, &inp(&w, &b), &t.loc(), "resume").unwrap();
        assert_eq!(r.snapshot(&w, &b), snaps[k]);
        for d in k + 1..=300 {
            let rep = r.advance_day(&b);
            assert_eq!(rep, reports[d - 1], "day {d} after resuming at {k}");
            assert_eq!(r.snapshot(&w, &b), snaps[d], "state after day {d}, resumed at {k}");
        }
        assert_eq!(r, cont);
    }
}

#[test]
fn loading_does_not_charge_re_add_or_re_refund_and_new_order_ids_do_not_collide() {
    use strategy_game::building::data::BuildingType::{Farm, Mine};
    use strategy_game::economy::map_state::{CancelError, CancelTarget, ConstructionOrderId};
    let (w, b) = (world(), buildings());
    let mut sim = make("economy_scenario_m4e.ron", &w, &b);
    let farm = sim.registry.request_build(&mut sim.countries, 1, strategy_game::world::MapStateId(20), Farm, &b).unwrap();
    let mine = sim.registry.request_build(&mut sim.countries, 1, strategy_game::world::MapStateId(20), Mine, &b).unwrap();
    for _ in 0..10 {
        sim.advance_day(&b);
    }
    sim.registry.cancel_order(&mut sim.countries, 1, CancelTarget::Order(mine)).unwrap();
    for _ in 0..25 {
        sim.advance_day(&b); // Farmは30日で完成
    }
    let t = Tmp::new("ids");
    save(&sim, &inp(&w, &b), &t.loc(), "ids").unwrap();
    let mut r = make("economy_scenario_m4e.ron", &w, &b);
    load(&mut r, &inp(&w, &b), &t.loc(), "ids").unwrap();
    assert_eq!(r.countries[0].treasury, sim.countries[0].treasury, "no cost is charged again");
    assert_eq!(r.registry.get(strategy_game::world::MapStateId(20)).unwrap().buildings[&Farm], sim.registry.get(strategy_game::world::MapStateId(20)).unwrap().buildings[&Farm]);
    for x in [&mut sim, &mut r] {
        let before = x.countries[0].treasury;
        assert_eq!(x.registry.cancel_order(&mut x.countries, 1, CancelTarget::Order(mine)).unwrap_err(), CancelError::AlreadyCancelled(mine));
        assert_eq!(x.registry.cancel_order(&mut x.countries, 1, CancelTarget::Order(farm)).unwrap_err(), CancelError::AlreadyCompleted(farm));
        assert_eq!(x.countries[0].treasury, before);
        let next: ConstructionOrderId = x.registry.request_build(&mut x.countries, 1, strategy_game::world::MapStateId(20), Mine, &b).unwrap();
        assert_eq!(next.seq, 3, "the next id continues from the saved high-water mark");
    }
    assert_eq!(r, sim);
}

// ---------------------------------------------------------------- 別プロセスでの再開

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_world_economy_check")).args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn day_lines(stdout: &str) -> Vec<String> {
    stdout.lines().filter(|l| l.starts_with("day ")).map(str::to_string).collect()
}

#[test]
fn a_separate_process_resumes_from_the_file_and_matches_the_continuous_run() {
    let t = Tmp::new("process");
    let base = t.0.to_string_lossy().to_string();
    // 保存日 = 100(途中)・30(再開の最初の日が月次の日)。
    for (save_days, rest) in [("100", "60"), ("30", "40")] {
        let total = (save_days.parse::<u32>().unwrap() + rest.parse::<u32>().unwrap()).to_string();
        let (ok, out, err) = cli(&["--economy-save", save_days, "--save-base", &base, "--name", "cli"]);
        assert!(ok, "save failed: {out}{err}");
        assert!(out.contains("保存成功"), "{out}");
        let (ok, resumed, err) = cli(&["--economy-resume", rest, "--save-base", &base, "--name", "cli"]);
        assert!(ok, "resume failed: {resumed}{err}");
        assert!(resumed.contains("復元成功"), "{resumed}");
        let (ok, cont, err) = cli(&["--economy-run", &total, "--digest-from", save_days]);
        assert!(ok, "continuous failed: {cont}{err}");
        let (a, b) = (day_lines(&resumed), day_lines(&cont));
        assert_eq!(a.len(), rest.parse::<usize>().unwrap());
        assert_eq!(a, b, "split at {save_days}: every resumed day equals the continuous run");
    }
    // 失敗時: ファイルが無い/壊れている/旧V1 → 非ゼロ終了で「復元失敗」を表示し、成功とは表示しない。
    let (ok, out, err) = cli(&["--economy-resume", "5", "--save-base", &base, "--name", "missing"]);
    assert!(!ok && !out.contains("復元成功") && err.contains("復元失敗"), "{out}{err}");
    t.write_raw("v1copy", &fs::read(V1_PATH).unwrap());
    let (ok, out, err) = cli(&["--economy-resume", "5", "--save-base", &base, "--name", "v1copy"]);
    assert!(!ok && !out.contains("復元成功") && err.contains("legacy V1"), "{out}{err}");
    // 引数なしの保存先は無い(通常起動・保存先未指定ではユーザーセーブを作らない)。
    let (ok, _, err) = cli(&["--economy-save", "5", "--name", "x"]);
    assert!(!ok && err.contains("--save-base"), "{err}");
}

#[test]
fn read_snapshot_reads_without_restoring() {
    let (w, b) = (world(), buildings());
    let t = Tmp::new("readonly");
    let sim = advanced(&w, &b, 33);
    save(&sim, &inp(&w, &b), &t.loc(), "r").unwrap();
    assert_eq!(read_snapshot(&t.loc(), "r").unwrap(), sim.snapshot(&w, &b));
    assert!(matches!(read_snapshot(&t.loc(), "nope").unwrap_err(), FileError::NotFound(_)));
}
