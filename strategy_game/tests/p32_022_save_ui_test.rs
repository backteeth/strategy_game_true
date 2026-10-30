//! P32-022 (S3): 経済セッション画面の保存・一覧・事前確認・復元(`hexmap::save_view` と `economy_view` のコマンド)のテスト。
//! 共有API(`EconomySession::{save, inspect, load, list_saves}`)を画面のコマンド経由で呼ぶ接続を確認する。
//! 前半はコマンド適用の純粋な関数(`apply_file_command`)、後半は実際の Bevy `App`(`MinimalPlugins` + 入力 + 実際のシステム)。
//! 描画(実ウィンドウ)・実マウス・実キーボード・OSのIMEは含まない。書込・負例はすべて一時ディレクトリで行い、`saves/` には触れない。

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use strategy_game::building::data::BuildingType;
use strategy_game::economy::map_economy_file::{
    self as mef, DirectoryStatus, EntryStatus, FileError, SaveLocation, decode, encode,
};
use strategy_game::economy::map_session::{EconomySession, GeographyData, SessionPaths, scenario_by_id};
use strategy_game::economy::map_state::ConstructionOrderId;
use strategy_game::economy::map_state::snapshot::MapEconomySnapshot;
use strategy_game::hexmap::build_view::{
    BuildButton, BuildPanelRoot, CancelButton, spawn_build_panel, sync_build_panel,
};
use strategy_game::hexmap::data::load_from_path;
use strategy_game::hexmap::economy_view::{
    EconomySessionCorePlugin, SessionCommand, SessionRes, SessionSet, SessionUi, session_buttons,
};
use strategy_game::hexmap::save_view::{
    FileCtx, LoadSaveButton, NAME_INPUT_MAX_CHARS, NameState, Preview, PreviewClass, RefreshSavesButton, SaveLocationRes,
    SaveNameField, SavePanelRoot, SaveRowButton, SaveSubmitButton, SaveUi, apply_file_command, file_error_text,
    initial_refresh, name_state, preview_model, row_text, save_buttons, save_text_input, spawn_save_panel,
    sync_save_panel, sync_save_panel_visibility, sync_save_texts,
};
use strategy_game::hexmap::view::{HexSelection, LoadedHexMap};
use strategy_game::hexmap::world_view::{LoadedWorld, catalog_names};
use strategy_game::world::MapStateId;
use strategy_game::world::load_world_preview;

fn geo() -> &'static GeographyData {
    static G: OnceLock<GeographyData> = OnceLock::new();
    G.get_or_init(|| GeographyData::load(&SessionPaths::default()).unwrap())
}

fn session() -> EconomySession {
    session_of("m4e")
}

fn session_of(id: &str) -> EconomySession {
    EconomySession::load_files(scenario_by_id(id).unwrap(), geo().model(), &SessionPaths::default()).unwrap()
}

fn snap(s: &EconomySession) -> MapEconomySnapshot {
    s.sim.snapshot(geo().model(), &s.buildings)
}

fn advance(s: &mut EconomySession, days: u32) {
    for _ in 0..days {
        s.advance_day();
    }
}

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let p = std::env::temp_dir().join(format!(
            "p32_022_{tag}_{}_{nanos}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn loc(&self) -> SaveLocation {
        SaveLocation::new(&self.0)
    }
    fn dir(&self) -> PathBuf {
        self.0.join("map_economy")
    }
    fn file(&self, name: &str) -> PathBuf {
        self.dir().join(format!("{name}.mapeco.ron"))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names_in(dir: &PathBuf) -> Vec<String> {
    let mut v: Vec<String> = match fs::read_dir(dir) {
        Ok(rd) => rd.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect(),
        Err(_) => Vec::new(),
    };
    v.sort();
    v
}

/// 画面を介さずにコマンドを適用する(`apply_session_commands` と同じ関数 `apply_file_command`)。
fn run(
    s: &mut EconomySession,
    actor: &mut Option<u32>,
    save: &mut SaveUi,
    loc: &SaveLocation,
    cmd: SessionCommand,
) -> strategy_game::hexmap::save_view::FileResult {
    let ctx = FileCtx { map: Some(&geo().map), world: Some(geo().model()), location: loc };
    apply_file_command(s, actor, save, &ctx, cmd)
}

fn save_cmd(actor: Option<u32>, name: &str, replace: bool) -> SessionCommand {
    SessionCommand::SaveAs { actor, name: name.to_string(), replace }
}

fn ok_text(r: &strategy_game::hexmap::save_view::FileResult) -> &str {
    let (ok, t) = r.text.as_ref().expect("a result line");
    assert!(*ok, "expected success: {t}");
    t
}

fn fail_text(r: &strategy_game::hexmap::save_view::FileResult) -> &str {
    let (ok, t) = r.text.as_ref().expect("a result line");
    assert!(!*ok, "expected failure: {t}");
    t
}

fn date_str(s: &EconomySession) -> String {
    let d = s.sim.date;
    format!("{:04}/{:02}/{:02}", d.year, d.month, d.day)
}

// ---------------------------------------------------------------- 保存

#[test]
fn a_save_through_the_ui_command_is_read_by_the_shared_api_and_changes_nothing() {
    let t = Tmp::new("save");
    let mut s = session();
    advance(&mut s, 3);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    let before = snap(&s);
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "first", false));
    let text = ok_text(&r).to_string();
    assert!(text.contains("first") && text.contains(&date_str(&s)) && text.contains("新規"), "{text}");
    assert!(!r.restored);
    assert_eq!(snap(&s), before, "saving does not change the economy, date or queues");
    assert_eq!(actor, Some(1));
    // 共有APIで読める。保存時点の日付・状態と一致する。
    let file = mef::read_file(&t.loc(), "first").unwrap();
    assert_eq!(file.snapshot, before);
    let rep = s.inspect(&geo().map, geo().model(), &t.loc(), "first").unwrap();
    assert!(rep.verdict.is_exact() && matches!(rep.restorable, Some(Ok(()))));
    assert_eq!(names_in(&t.dir()), vec!["first.mapeco.ron".to_string()], "no temporary file is left");
    // 一覧にも出る(読み取りのみ)。
    assert_eq!(ui.list.as_ref().unwrap().entries.len(), 1);
}

#[test]
fn saving_needs_an_explicit_current_actor_and_never_writes_otherwise() {
    let t = Tmp::new("actor");
    let mut s = session();
    let before = snap(&s);
    let mut ui = SaveUi::default();
    // 未選択
    let mut actor = None;
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(None, "a", false));
    assert!(fail_text(&r).contains("操作国家が未選択"));
    // 押した時点の操作主体と現在が違う
    let mut actor = Some(1);
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(2), "a", false));
    assert!(fail_text(&r).contains("操作国家が変わっています"));
    // AI国家
    let mut actor = Some(3);
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(3), "a", false));
    assert!(fail_text(&r).contains("AI国家"));
    assert!(!t.dir().exists(), "nothing was created");
    assert_eq!(snap(&s), before);
}

#[test]
fn invalid_names_and_outside_paths_are_refused_with_a_reason_and_nothing_is_created() {
    let t = Tmp::new("names");
    let mut s = session();
    let mut actor = Some(1);
    let mut ui = SaveUi::default();
    let long = "a".repeat(65);
    let bad = [
        "", "a b", "../x", "..\\x", "x.mapeco.ron", "a/b", "C:x", "con", "NUL", "COM1", "あ", "a.b", long.as_str(),
        "saves/../x", "/abs",
    ];
    for name in bad {
        let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), name, false));
        let text = fail_text(&r).to_string();
        assert!(text.contains("保存できません"), "{name:?}: {text}");
        if !name.is_empty() {
            assert!(text.contains("保存名が使えません"), "{name:?}: {text}");
        }
        // 表示用の判定も共有仕様と一致する。
        let st = name_state(name, None, &t.loc());
        assert!(matches!(st, NameState::Empty | NameState::Invalid(_)), "{name:?}");
        assert!(t.loc().file_path(name).is_err());
    }
    assert!(!t.dir().exists(), "no directory or file was created for a refused name");
    assert_eq!(names_in(&t.0), Vec::<String>::new());
    // 旧V1のスロットは名前としては通っても、保存先は map_economy 配下の .mapeco.ron だけ。
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "savegame_v1", false));
    ok_text(&r);
    assert_eq!(names_in(&t.dir()), vec!["savegame_v1.mapeco.ron".to_string()]);
    assert!(!t.0.join("savegame_v1.ron").exists(), "the legacy V1 slot is never written");
    const { assert!(NAME_INPUT_MAX_CHARS >= 64) };
    assert!(file_error_text(&FileError::InvalidName("x".into())).contains("英数字"));
}

#[test]
fn replacing_a_save_is_explicit_and_never_silently_goes_to_another_file() {
    let t = Tmp::new("replace");
    let mut s = session();
    let mut actor = Some(1);
    let mut ui = SaveUi::default();
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "slot", false)));
    let first = fs::read(t.file("slot")).unwrap();
    advance(&mut s, 5);
    // 明示なしの同名保存は拒否され、ファイルは変わらない。
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "slot", false));
    assert!(fail_text(&r).contains("すでにあります") && fail_text(&r).contains("置換"));
    assert_eq!(fs::read(t.file("slot")).unwrap(), first);
    // 大文字小文字だけ違う名前は、別ファイルへも置換へも黙って進まない。
    for variant in ["SLOT", "Slot"] {
        let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), variant, true));
        assert!(fail_text(&r).contains("大文字小文字"), "{variant}");
    }
    assert_eq!(names_in(&t.dir()), vec!["slot.mapeco.ron".to_string()]);
    assert!(matches!(name_state("SLOT", ui.list.as_ref(), &t.loc()), NameState::CaseConflict(ref o) if o == "slot"));
    assert_eq!(name_state("slot", ui.list.as_ref(), &t.loc()), NameState::Replace);
    assert_eq!(name_state("other", ui.list.as_ref(), &t.loc()), NameState::New);
    // 明示した置換は成功し、置換対象が結果に出る。
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "slot", true));
    let text = ok_text(&r).to_string();
    assert!(text.contains("既存を置換") && text.contains(&date_str(&s)), "{text}");
    assert_ne!(fs::read(t.file("slot")).unwrap(), first);
    assert_eq!(mef::read_snapshot(&t.loc(), "slot").unwrap(), snap(&s));
}

#[test]
fn a_save_over_a_file_that_is_not_a_map_economy_file_is_refused_and_leaves_it_untouched() {
    let t = Tmp::new("overwrite");
    fs::create_dir_all(t.dir()).unwrap();
    fs::write(t.file("odd"), "not a save").unwrap();
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "odd", true));
    assert!(fail_text(&r).contains("上書きしませんでした"));
    assert_eq!(fs::read_to_string(t.file("odd")).unwrap(), "not a save");
}

// ---------------------------------------------------------------- 一覧

#[test]
fn listing_a_missing_directory_says_no_saves_and_creates_nothing() {
    let t = Tmp::new("missing");
    let mut s = session();
    let (mut actor, mut ui) = (None, SaveUi::default());
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::RefreshSaves);
    ok_text(&r);
    let l = ui.list.as_ref().unwrap();
    assert_eq!(l.directory, DirectoryStatus::Missing);
    assert!(l.entries.is_empty());
    assert!(!t.dir().exists(), "listing never creates the save directory");
    assert_eq!(names_in(&t.0), Vec::<String>::new());
}

fn write_file(t: &Tmp, name: &str, content: &str) {
    fs::create_dir_all(t.dir()).unwrap();
    fs::write(t.file(name), content).unwrap();
}

#[test]
fn the_list_distinguishes_valid_broken_legacy_and_unsupported_files_without_touching_them() {
    let t = Tmp::new("list");
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "good", false)));
    let good = fs::read_to_string(t.file("good")).unwrap();
    write_file(&t, "broken", "(kind: \"map_economy_state\", format_version: 2, trunc");
    write_file(&t, "v1like", "(version: 1, game_time: (year: 1800))");
    write_file(&t, "ver9", &good.replace("format_version: 2", "format_version: 9"));
    write_file(&t, "ver1", &good.replace("format_version: 2", "format_version: 1"));
    write_file(&t, "otherkind", &good.replace("map_economy_state", "something_else"));
    fs::write(t.dir().join("notes.txt"), "x").unwrap();
    fs::write(t.dir().join("leftover.mapeco.ron.tmp"), "x").unwrap();
    let before: Vec<(String, Vec<u8>)> =
        names_in(&t.dir()).into_iter().map(|n| (n.clone(), fs::read(t.dir().join(&n)).unwrap())).collect();

    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::RefreshSaves));
    let l = ui.list.as_ref().unwrap();
    let order: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(order, ["broken", "good", "otherkind", "v1like", "ver1", "ver9"], "stable name order; unrelated files are not listed");
    for e in &l.entries {
        let (text, selectable) = row_text(e);
        match e.name.as_str() {
            "good" => {
                assert!(matches!(e.status, EntryStatus::Valid { .. }) && selectable);
                assert!(text.contains("形式は正常") && text.contains(&date_str(&s)));
            }
            n => {
                assert!(matches!(e.status, EntryStatus::Unusable(_)), "{n}");
                assert!(!selectable, "{n}: an unusable file cannot be selected");
                assert!(text.contains("選択できません"), "{n}: {text}");
                assert!(!text.contains("形式は正常"), "{n}: a broken file is never shown as a normal save");
            }
        }
    }
    let text_of = |n: &str| row_text(l.entries.iter().find(|e| e.name == n).unwrap()).0;
    assert!(text_of("broken").contains("破損"));
    assert!(text_of("v1like").contains("旧ゲームのセーブ(V1)"));
    assert!(text_of("ver9").contains("非対応"));
    assert!(text_of("ver1").contains("バージョン1"));
    assert!(text_of("otherkind").contains("別の種類"));
    let after: Vec<(String, Vec<u8>)> =
        names_in(&t.dir()).into_iter().map(|n| (n.clone(), fs::read(t.dir().join(&n)).unwrap())).collect();
    assert_eq!(before, after, "listing repairs, converts and deletes nothing");
}

#[test]
fn the_selection_follows_the_name_not_the_row_and_is_invalidated_when_the_file_disappears() {
    let t = Tmp::new("select");
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    for n in ["bbb", "ccc"] {
        ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), n, false)));
    }
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "ccc".into() }));
    assert_eq!(ui.selected.as_deref(), Some("ccc"));
    assert_eq!(ui.preview.as_ref().unwrap().name, "ccc");
    // 先頭に新しいファイルが入って順番が変わっても、選択と確認結果は ccc のまま。
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "aaa", false)));
    let order: Vec<&str> = ui.list.as_ref().unwrap().entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(order, ["aaa", "bbb", "ccc"]);
    assert_eq!(ui.selected.as_deref(), Some("ccc"));
    assert_eq!(ui.preview.as_ref().unwrap().name, "ccc");
    // ファイルが内容ごと変わったら、古い確認結果は残さない。
    fs::write(t.file("ccc"), "(kind: \"map_economy_state\"").unwrap();
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::RefreshSaves));
    assert!(ui.preview.is_none(), "the preview of a changed file is not shown as current");
    // 消えたら選択も無効。
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "bbb".into() }));
    fs::remove_file(t.file("bbb")).unwrap();
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::RefreshSaves));
    assert!(ui.selected.is_none() && ui.preview.is_none());
}

// ---------------------------------------------------------------- 事前確認・復元

/// 保存ファイルの識別情報の1項目を書き換えたファイルを作る(静的入力が保存時と違う状況の再現)。
fn tamper_item(t: &Tmp, from: &str, to: &str, key: &str) {
    let text = fs::read_to_string(t.file(from)).unwrap();
    let mut d = decode(&text).unwrap();
    let item = d.identity.items.iter_mut().find(|i| i.key == key).unwrap();
    let first = if item.digest.starts_with('0') { '1' } else { '0' };
    item.digest = format!("{first}{}", &item.digest[1..]);
    fs::write(t.file(to), encode(&d.snapshot, &d.identity).unwrap()).unwrap();
}

fn inspect_and_load(
    s: &mut EconomySession,
    actor: &mut Option<u32>,
    ui: &mut SaveUi,
    t: &Tmp,
    name: &str,
) -> (strategy_game::hexmap::save_view::FileResult, strategy_game::hexmap::save_view::FileResult) {
    let a = run(s, actor, ui, &t.loc(), SessionCommand::InspectSave { name: name.into() });
    let b = run(s, actor, ui, &t.loc(), SessionCommand::LoadSave { actor: *actor, name: name.into() });
    (a, b)
}

#[test]
fn inspect_shows_exact_warning_and_rejection_apart_and_changes_nothing() {
    let t = Tmp::new("classes");
    let mut s = session();
    advance(&mut s, 4);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "exact", false)));
    tamper_item(&t, "exact", "warned", "land_terrain");
    tamper_item(&t, "exact", "rejected", "grid");
    let before = snap(&s);
    let sel_before = ui.selected.clone();

    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "exact".into() }));
    let m = preview_model(ui.preview.as_ref().unwrap());
    assert_eq!(m.class, PreviewClass::Ok);
    assert!(m.title.contains("完全一致") && m.lines.iter().any(|l| l.contains(&date_str(&s))), "{m:?}");
    assert!(m.lines.iter().any(|l| l.contains("ステート") && l.contains("国家") && l.contains("建設キュー")));

    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "warned".into() }));
    let m = preview_model(ui.preview.as_ref().unwrap());
    assert_eq!(m.class, PreviewClass::Warn, "{m:?}");
    assert!(m.title.contains("警告つき") && m.lines.iter().any(|l| l.contains("警告") && l.contains("地形")));
    assert!(m.detail.iter().any(|d| d.contains("land_terrain")), "internal names are only in the details");
    assert!(!m.lines.iter().any(|l| l.contains("land_terrain")), "no internal item name in the normal view");

    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "rejected".into() });
    fail_text(&r);
    let m = preview_model(ui.preview.as_ref().unwrap());
    assert_eq!(m.class, PreviewClass::Reject, "{m:?}");
    assert!(m.lines.iter().any(|l| l.contains("拒否") && l.contains("地図の格子")), "{m:?}");

    assert_eq!(snap(&s), before, "inspecting restores nothing and changes no economic state");
    assert_eq!(actor, Some(1));
    let _ = sel_before;
    // 拒否の確認結果からは、復元コマンドを送っても突破できない。共有 `load` も同じ理由で拒否する。
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "rejected".into() });
    assert!(fail_text(&r).contains("拒否"));
    assert!(!r.restored);
    let direct = s.load(&geo().map, geo().model(), &t.loc(), "rejected");
    assert!(matches!(direct, Err(FileError::Incompatible(_))));
    assert_eq!(snap(&s), before);
}

#[test]
fn a_warning_save_restores_after_the_warning_is_shown_and_keeps_the_saved_dynamic_state() {
    let t = Tmp::new("warnload");
    let mut s = session();
    advance(&mut s, 4);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "exact", false)));
    let saved = snap(&s);
    tamper_item(&t, "exact", "warned", "climate");
    advance(&mut s, 9);
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "warned");
    ok_text(&a);
    let text = ok_text(&b).to_string();
    assert!(b.restored && text.contains("警告つきで復元") && text.contains("気候"), "{text}");
    assert_eq!(snap(&s), saved, "dynamic state comes back exactly as saved");
}

#[test]
fn a_restore_needs_a_fresh_preview_and_a_changed_economy_invalidates_it() {
    let t = Tmp::new("fresh");
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "x", false)));
    advance(&mut s, 2);
    let before = snap(&s);
    // 確認なし
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "x".into() });
    assert!(fail_text(&r).contains("事前確認"));
    // 別の保存の確認では復元できない。
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "y", false)));
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "y".into() }));
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "x".into() });
    assert!(fail_text(&r).contains("事前確認"));
    // 確認後に経済状態が変わったら、確認は現在の結果として使えない。
    ui.state_changed();
    assert!(ui.fresh_preview("y").is_none() && ui.preview.is_none());
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "y".into() });
    assert!(fail_text(&r).contains("事前確認"));
    assert_eq!(snap(&s), before);
    // 操作国家の違うコマンドは実行しない。
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "x".into() }));
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(2), name: "x".into() });
    assert!(fail_text(&r).contains("操作国家が変わっています"));
    let mut none = None;
    let r = run(&mut s, &mut none, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: None, name: "x".into() });
    assert!(fail_text(&r).contains("未選択"));
    assert_eq!(snap(&s), before);
}

#[test]
fn the_shared_load_revalidates_after_the_preview_and_a_failure_keeps_every_state() {
    let t = Tmp::new("revalidate");
    let mut s = session();
    advance(&mut s, 2);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "x", false)));
    advance(&mut s, 6);
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "x".into() }));
    assert_eq!(preview_model(ui.preview.as_ref().unwrap()).class, PreviewClass::Ok);
    ui.selected = Some("x".into());
    let before = snap(&s);
    // 確認の後でファイルが壊れた: 確認結果の許可に頼らず、共有 load が失敗する。
    fs::write(t.file("x"), "(kind: \"map_economy_state\", format_version: 2, trunc").unwrap();
    // (一覧上の状態が変わるので、確認は一覧更新で無効になりうる。ここでは更新前の確認結果のまま復元を試みる)
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "x".into() });
    let text = fail_text(&r).to_string();
    assert!(text.contains("復元に失敗") && text.contains("変更していません"), "{text}");
    assert!(!r.restored);
    assert_eq!(snap(&s), before, "economy state, date and queues are kept");
    assert_eq!(actor, Some(1));
    assert!(ui.preview.is_none(), "the old preview is dropped after a failed restore");
    // 確認の後で識別情報が不一致になった場合も同様。
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "z", false)));
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::InspectSave { name: "z".into() }));
    tamper_item(&t, "z", "z", "scenario_structure");
    let r = run(&mut s, &mut actor, &mut ui, &t.loc(), SessionCommand::LoadSave { actor: Some(1), name: "z".into() });
    assert!(fail_text(&r).contains("一致しないため復元できません"));
    assert_eq!(snap(&s), before);
}

#[test]
fn a_save_from_another_scenario_is_refused_and_keeps_the_running_session() {
    let t = Tmp::new("scenario");
    let mut d = session_of("m4d");
    advance(&mut d, 3);
    d.save(&geo().map, geo().model(), &t.loc(), "from_m4d").unwrap();
    let mut s = session();
    advance(&mut s, 2);
    let before = snap(&s);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "from_m4d");
    let _ = a;
    fail_text(&b);
    assert!(!b.restored);
    assert_eq!(snap(&s), before);
    assert_eq!(preview_model(ui.preview.as_ref().map_or(&dummy_preview(), |p| p)).class, PreviewClass::Reject);
}

fn dummy_preview() -> Preview {
    Preview { name: String::new(), gen_at: 0, entry: None, result: Err(FileError::NotFound(PathBuf::new())) }
}

#[test]
fn a_successful_restore_replaces_the_state_stops_time_and_reports_the_dates() {
    let t = Tmp::new("restore");
    let mut s = session();
    advance(&mut s, 12);
    let saved = snap(&s);
    let saved_date = date_str(&s);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "r", false)));
    advance(&mut s, 40);
    ui.state_changed();
    assert_ne!(snap(&s), saved);
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "r");
    ok_text(&a);
    let text = ok_text(&b).to_string();
    assert!(b.restored);
    assert_eq!(snap(&s), saved);
    assert!(text.contains(&saved_date) && text.contains("次に処理する日") && text.contains("停止中"), "{text}");
    assert_eq!(date_str(&s), saved_date, "the same processed day is not run again right after the restore");
    assert_eq!(actor, Some(1), "a still-valid actor is kept");
    assert!(ui.preview.is_none(), "the old preview is cleared");
    assert_eq!(ui.restore_epoch, 1);
}

// ---------------------------------------------------------------- 保存・復元・継続の一致

#[test]
fn save_restore_and_continue_equals_the_continuous_run_including_construction() {
    let t = Tmp::new("continue");
    let mut direct = session();
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    // 発注(支払い) → 10日 → 途中保存 → さらに進行(完成を含む) → 保存時点へ復元 → 継続
    for sess in [&mut direct, &mut s] {
        sess.request_build(Some(1), MapStateId(20), BuildingType::Farm).unwrap();
        sess.request_build(Some(1), MapStateId(20), BuildingType::Mine).unwrap();
        advance(sess, 10);
    }
    let treasury_at_save = s.country(1).unwrap().treasury;
    let queue_at_save: Vec<_> = s.country(1).unwrap().construction_queue.iter().map(|o| (o.id, o.progress)).collect();
    assert_eq!(queue_at_save.len(), 2);
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "mid", false)));
    advance(&mut s, 45); // Farm(30日)が完成し、Mine(60日)は途中
    assert_ne!(s.country(1).unwrap().construction_queue.len(), 2);
    ui.state_changed();
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "mid");
    ok_text(&a);
    ok_text(&b);
    assert_eq!(s.country(1).unwrap().treasury, treasury_at_save, "the payment is not repeated or lost");
    let restored_queue: Vec<_> = s.country(1).unwrap().construction_queue.iter().map(|o| (o.id, o.progress)).collect();
    assert_eq!(restored_queue, queue_at_save, "orders and their progress come back");
    // 継続は連続実行と一致する(完成・資金・月次を含む)。
    advance(&mut s, 100);
    advance(&mut direct, 100);
    assert_eq!(snap(&s), snap(&direct));
    // 完成レベルは1回分だけ。
    let farm_level = s.state_economy(MapStateId(20)).unwrap().buildings.get(&BuildingType::Farm).copied().unwrap_or(0);
    let farm_direct = direct.state_economy(MapStateId(20)).unwrap().buildings.get(&BuildingType::Farm).copied().unwrap_or(0);
    assert_eq!(farm_level, farm_direct);
}

#[test]
fn a_cancel_after_a_restore_refunds_once_and_matches_the_direct_run() {
    let t = Tmp::new("cancel");
    let mut s = session();
    let mut direct = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    let id: ConstructionOrderId = s.request_build(Some(1), MapStateId(20), BuildingType::Mine).unwrap();
    direct.request_build(Some(1), MapStateId(20), BuildingType::Mine).unwrap();
    advance(&mut s, 5);
    advance(&mut direct, 5);
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "c", false)));
    s.cancel_order(Some(1), id).unwrap(); // 復元前に取消し(返金済み)
    advance(&mut s, 3);
    ui.state_changed();
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "c");
    ok_text(&a);
    ok_text(&b);
    let before = s.country(1).unwrap().treasury;
    let r = s.cancel_order(Some(1), id).unwrap();
    assert_eq!(s.country(1).unwrap().treasury, before + r.refund, "refunded exactly once after the restore");
    assert!(s.cancel_order(Some(1), id).is_err(), "a second cancel is refused, no double refund");
    // 直接実行でも同じ結果。
    direct.cancel_order(Some(1), id).unwrap();
    assert_eq!(s.country(1).unwrap().treasury, direct.country(1).unwrap().treasury);
}

// ---------------------------------------------------------------- 別プロセス(CLI)との往復

fn cli(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_world_economy_check")).args(args).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn a_file_saved_by_the_cli_restores_through_the_ui_path_and_continues_like_a_direct_run() {
    let t = Tmp::new("cli");
    let base = t.0.to_string_lossy().to_string();
    let (ok, out, err) =
        cli(&["--economy-save", "40", "--scenario", "m4e", "--name", "from_cli", "--save-base", &base]);
    assert!(ok && out.contains("保存成功"), "{out}{err}");
    let mut s = session();
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    let (a, b) = inspect_and_load(&mut s, &mut actor, &mut ui, &t, "from_cli");
    ok_text(&a);
    ok_text(&b);
    let mut direct = session();
    advance(&mut direct, 40);
    assert_eq!(snap(&s), snap(&direct), "the CLI file restores to the exact state of a 40-day run");
    advance(&mut s, 60);
    advance(&mut direct, 60);
    assert_eq!(snap(&s), snap(&direct));
}

#[test]
fn a_file_saved_by_the_ui_is_inspected_by_the_cli_with_the_same_date() {
    let t = Tmp::new("ui2cli");
    let base = t.0.to_string_lossy().to_string();
    let mut s = session();
    advance(&mut s, 33);
    let (mut actor, mut ui) = (Some(1), SaveUi::default());
    ok_text(&run(&mut s, &mut actor, &mut ui, &t.loc(), save_cmd(Some(1), "from_ui", false)));
    let (ok, out, err) = cli(&["--economy-inspect", "--scenario", "m4e", "--name", "from_ui", "--save-base", &base]);
    assert!(ok, "{out}{err}");
    let d = s.sim.date;
    assert!(out.contains(&format!("{}/{:02}/{:02}", d.year, d.month, d.day)), "{out}");
    assert!(!out.trim().is_empty());
}

// ---------------------------------------------------------------- Bevy App(実際のシステム)

struct World2 {
    map: strategy_game::hexmap::data::HexMap,
    loaded: LoadedWorld,
}

fn world2() -> World2 {
    let p = SessionPaths::default();
    let map = load_from_path(&p.map).unwrap();
    let data = load_world_preview(&p.provinces, &p.states, &map).unwrap();
    let names = catalog_names(&data);
    World2 { map, loaded: LoadedWorld { data, names } }
}

#[derive(Resource, Default)]
struct KeyProbe {
    g_reached_map: u32,
    one_reached_map: u32,
    escape_reached_map: u32,
}

fn probe(keys: Res<ButtonInput<KeyCode>>, mut p: ResMut<KeyProbe>) {
    p.g_reached_map += keys.just_pressed(KeyCode::KeyG) as u32;
    p.one_reached_map += keys.just_pressed(KeyCode::Digit1) as u32;
    p.escape_reached_map += keys.just_pressed(KeyCode::Escape) as u32;
}

/// 実際のシステム: 入力 → ボタン → 適用 → 表示の作り直し。
fn ui_app(loc: SaveLocation, session: EconomySession) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).add_plugins(bevy::input::InputPlugin);
    app.add_plugins(EconomySessionCorePlugin);
    let w = world2();
    app.insert_resource(LoadedHexMap(w.map)).insert_resource(w.loaded);
    app.insert_resource(HexSelection(Some(1234)));
    app.insert_resource(SaveLocationRes(loc));
    app.insert_resource(SessionRes { session });
    app.init_resource::<KeyProbe>();
    app.add_systems(Update, save_text_input.before(SessionSet::Input));
    app.add_systems(Update, probe.after(save_text_input).before(SessionSet::Input));
    app.add_systems(Update, (session_buttons, save_buttons, initial_refresh).in_set(SessionSet::Input));
    app.add_systems(
        Update,
        (sync_build_panel, sync_save_panel, sync_save_texts, sync_save_panel_visibility).in_set(SessionSet::Sync),
    );
    let mut commands = app.world_mut().commands();
    spawn_build_panel(&mut commands);
    spawn_save_panel(&mut commands);
    app.world_mut().flush();
    app.update();
    app
}

fn send(app: &mut App, cmd: SessionCommand) {
    app.world_mut().resource_mut::<Messages<SessionCommand>>().write(cmd);
}

fn snap_app(app: &App) -> MapEconomySnapshot {
    snap(&app.world().resource::<SessionRes>().session)
}

fn log_text(app: &App) -> String {
    app.world().resource::<SessionUi>().log.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>().join("\n")
}

fn press_entity(app: &mut App, e: Entity) {
    *app.world_mut().get_mut::<Interaction>(e).unwrap() = Interaction::Pressed;
    app.update();
}

fn key(app: &mut App, code: KeyCode, logical: Key, text: Option<&str>) {
    app.world_mut().resource_mut::<Messages<KeyboardInput>>().write(KeyboardInput {
        key_code: code,
        logical_key: logical,
        state: ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    // 離す(押しっぱなしだと次の押下が just_pressed にならない)。
    app.world_mut().resource_mut::<Messages<KeyboardInput>>().write(KeyboardInput {
        key_code: code,
        logical_key: Key::Dead(None),
        state: ButtonState::Released,
        text: None,
        repeat: false,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

fn type_char(app: &mut App, c: char) {
    let (code, s) = match c {
        'a' => (KeyCode::KeyA, "a"),
        'b' => (KeyCode::KeyB, "b"),
        'g' => (KeyCode::KeyG, "g"),
        '1' => (KeyCode::Digit1, "1"),
        _ => panic!("unsupported test key"),
    };
    key(app, code, Key::Character(s.into()), Some(s));
}

fn click_left(app: &mut App) {
    app.world_mut().resource_mut::<Messages<MouseButtonInput>>().write(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window: Entity::PLACEHOLDER,
    });
    app.update();
    app.world_mut().resource_mut::<Messages<MouseButtonInput>>().write(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Released,
        window: Entity::PLACEHOLDER,
    });
    app.update();
}

fn find<C: Component>(app: &mut App) -> Vec<Entity> {
    let mut q = app.world_mut().query_filtered::<Entity, With<C>>();
    q.iter(app.world()).collect()
}

fn set_actor(app: &mut App, a: u32) {
    send(app, SessionCommand::SetActor(Some(a)));
    app.update();
}

#[test]
fn startup_and_listing_create_no_file_or_directory() {
    let t = Tmp::new("startup");
    let mut app = ui_app(t.loc(), session());
    for _ in 0..5 {
        app.update();
    }
    send(&mut app, SessionCommand::RefreshSaves);
    send(&mut app, SessionCommand::ToggleSavePanel);
    send(&mut app, SessionCommand::ToggleSavePanel);
    send(&mut app, SessionCommand::ToggleBuildPanel);
    app.update();
    assert!(app.world().resource::<SaveUi>().list.is_some(), "the initial refresh ran");
    assert_eq!(app.world().resource::<SaveUi>().list.as_ref().unwrap().directory, DirectoryStatus::Missing);
    assert!(!t.dir().exists());
    assert_eq!(names_in(&t.0), Vec::<String>::new());
    // 画面には「保存なし」と出る。
    let texts = all_texts(&mut app);
    assert!(texts.contains("保存なし"), "{texts}");
}

fn all_texts(app: &mut App) -> String {
    let mut q = app.world_mut().query::<&Text>();
    q.iter(app.world()).map(|t| t.0.clone()).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_name_field_takes_focus_edits_and_blocks_map_shortcuts_and_exit() {
    let t = Tmp::new("input");
    let mut app = ui_app(t.loc(), session());
    // フォーカスが無い間は、キーは地図のショートカットへ届き、入力欄は変わらない。
    type_char(&mut app, 'g');
    assert_eq!(app.world().resource::<KeyProbe>().g_reached_map, 1);
    assert_eq!(app.world().resource::<SaveUi>().name, "");
    // 入力欄をクリックしてフォーカス。
    let field = find::<SaveNameField>(&mut app)[0];
    *app.world_mut().get_mut::<Interaction>(field).unwrap() = Interaction::Hovered;
    click_left(&mut app);
    assert!(app.world().resource::<SaveUi>().focus);
    let (g0, d0, e0) = {
        let p = app.world().resource::<KeyProbe>();
        (p.g_reached_map, p.one_reached_map, p.escape_reached_map)
    };
    type_char(&mut app, 'a');
    type_char(&mut app, 'g');
    type_char(&mut app, '1');
    type_char(&mut app, 'b');
    assert_eq!(app.world().resource::<SaveUi>().name, "ag1b");
    key(&mut app, KeyCode::Backspace, Key::Backspace, None);
    assert_eq!(app.world().resource::<SaveUi>().name, "ag1");
    {
        let p = app.world().resource::<KeyProbe>();
        assert_eq!((p.g_reached_map, p.one_reached_map), (g0, d0), "typed keys never reach the map shortcuts");
    }
    // Esc は終了へ伝えず、フォーカスを外す。
    key(&mut app, KeyCode::Escape, Key::Escape, None);
    assert!(!app.world().resource::<SaveUi>().focus);
    assert_eq!(app.world().resource::<KeyProbe>().escape_reached_map, e0, "Esc is consumed by the field");
    // フォーカスを外した後はまた届く。
    type_char(&mut app, 'g');
    assert_eq!(app.world().resource::<KeyProbe>().g_reached_map, g0 + 1);
    assert_eq!(app.world().resource::<SaveUi>().name, "ag1");
    // 入力欄の外のクリックでもフォーカスが外れる。
    *app.world_mut().get_mut::<Interaction>(field).unwrap() = Interaction::Hovered;
    click_left(&mut app);
    assert!(app.world().resource::<SaveUi>().focus);
    *app.world_mut().get_mut::<Interaction>(field).unwrap() = Interaction::None;
    click_left(&mut app);
    assert!(!app.world().resource::<SaveUi>().focus);
    // 長すぎる入力は上限で止まる。
    app.world_mut().resource_mut::<SaveUi>().focus = true;
    for _ in 0..(NAME_INPUT_MAX_CHARS + 5) {
        type_char(&mut app, 'a');
    }
    assert_eq!(app.world().resource::<SaveUi>().name.chars().count(), NAME_INPUT_MAX_CHARS);
    // パネルが入力を地図へ透過しない構成: パネルと入力欄は Button(Interaction を発行)。
    assert!(find::<SavePanelRoot>(&mut app).iter().all(|e| app.world().get::<Button>(*e).is_some()));
    assert!(app.world().get::<Button>(field).is_some());
    assert!(!find::<BuildPanelRoot>(&mut app).is_empty());
}

#[test]
fn buttons_save_list_inspect_and_restore_through_the_real_systems_and_never_repeat() {
    let t = Tmp::new("flow");
    let mut app = ui_app(t.loc(), session());
    let sel_before = app.world().resource::<HexSelection>().0;
    // 操作国家が未選択: 保存ボタンは拒否される(ファイルは作らない)。
    app.world_mut().resource_mut::<SaveUi>().name = "flow".into();
    let submit = find::<SaveSubmitButton>(&mut app)[0];
    press_entity(&mut app, submit);
    assert!(log_text(&app).contains("操作国家が未選択"), "{}", log_text(&app));
    assert!(!t.dir().exists());
    // 国家を選び、進めてから保存。押し続けても(Changedが出ないので)重複しない。
    set_actor(&mut app, 1);
    send(&mut app, SessionCommand::AdvanceDays(7));
    app.update();
    let saved = snap_app(&app);
    *app.world_mut().get_mut::<Interaction>(submit).unwrap() = Interaction::None;
    app.update();
    press_entity(&mut app, submit);
    app.update();
    app.update();
    let n_saved = log_text(&app).matches("保存しました").count();
    assert_eq!(n_saved, 1, "{}", log_text(&app));
    assert!(!log_text(&app).contains("すでにあります"), "{}", log_text(&app));
    assert_eq!(snap_app(&app), saved);
    assert_eq!(mef::read_snapshot(&t.loc(), "flow").unwrap(), saved);
    // 一覧に出る。行は保存名を持つ。
    let rows = find::<SaveRowButton>(&mut app);
    assert_eq!(rows.len(), 1);
    assert_eq!(app.world().get::<SaveRowButton>(rows[0]).unwrap().name, "flow");
    // 同名の保存が見えるので、ボタンは「置換して保存」になり、押すと replace=true で送る。
    let texts = all_texts(&mut app);
    assert!(texts.contains("「flow」を置換して保存") && texts.contains("置換します"), "{texts}");
    // さらに進めてから、行を押して事前確認(経済状態を変えない)。
    send(&mut app, SessionCommand::AdvanceDays(30));
    app.update();
    let advanced = snap_app(&app);
    assert_ne!(advanced, saved);
    let rows = find::<SaveRowButton>(&mut app);
    press_entity(&mut app, rows[0]);
    assert_eq!(snap_app(&app), advanced, "the preview changes no economic state");
    assert_eq!(app.world().resource::<HexSelection>().0, sel_before, "the preview does not touch the map selection");
    let texts = all_texts(&mut app);
    assert!(texts.contains("復元できます") && texts.contains("を復元"), "{texts}");
    // 復元ボタン。
    let load = find::<LoadSaveButton>(&mut app);
    assert_eq!(load.len(), 1);
    press_entity(&mut app, load[0]);
    app.update();
    assert_eq!(snap_app(&app), saved, "the saved moment comes back");
    assert_eq!(app.world().resource::<SessionUi>().actor, Some(1));
    assert_eq!(app.world().resource::<HexSelection>().0, sel_before, "the map selection is kept");
    let log = log_text(&app);
    assert!(log.contains("復元しました") && !log.contains("保存しました"), "old results are cleaned: {log}");
    assert_eq!(log.matches("復元しました").count(), 1, "a held button does not restore twice");
    // 復元直後に時間は進まない。
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(snap_app(&app), saved);
    // 更新ボタンも1回だけ。
    let refresh = find::<RefreshSavesButton>(&mut app)[0];
    press_entity(&mut app, refresh);
    assert!(log_text(&app).contains("保存一覧を更新しました"));
}

#[test]
fn the_rest_of_the_batch_is_discarded_after_a_restore() {
    let t = Tmp::new("batch");
    let mut app = ui_app(t.loc(), session());
    set_actor(&mut app, 1);
    send(&mut app, SessionCommand::AdvanceDays(2));
    app.update();
    send(&mut app, save_cmd(Some(1), "b", false));
    app.update();
    let saved = snap_app(&app);
    send(&mut app, SessionCommand::AdvanceDays(3));
    app.update();
    send(&mut app, SessionCommand::InspectSave { name: "b".into() });
    app.update();
    // 同じフレームに受け付けた: 復元 → 日付進行 → 発注 → 取消し → 保存。復元より後のものは適用されない。
    send(&mut app, SessionCommand::LoadSave { actor: Some(1), name: "b".into() });
    send(&mut app, SessionCommand::AdvanceDays(5));
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: BuildingType::Farm });
    send(&mut app, SessionCommand::Cancel { actor: 1, order: ConstructionOrderId { country: 1, seq: 1 } });
    send(&mut app, save_cmd(Some(1), "after", true));
    app.update();
    assert_eq!(snap_app(&app), saved, "no stale command touched the restored state");
    assert!(!t.file("after").exists(), "a stale save is not written either");
    let log = log_text(&app);
    assert!(log.contains("4 件") && log.contains("破棄"), "{log}");
    // コマンドが復元より前にあった場合は、受け付けた順に適用される(復元で上書きされる)。
    send(&mut app, SessionCommand::AdvanceDays(1));
    send(&mut app, SessionCommand::InspectSave { name: "b".into() });
    app.update();
    assert_ne!(snap_app(&app), saved);
}

#[test]
fn old_buttons_and_old_order_ids_cannot_act_on_the_restored_state() {
    let t = Tmp::new("epoch");
    let mut app = ui_app(t.loc(), session());
    set_actor(&mut app, 1);
    app.world_mut().resource_mut::<HexSelection>().0 = Some(1234);
    send(&mut app, save_cmd(Some(1), "e", false));
    app.update();
    let saved = snap_app(&app);
    // 復元前の状態で発注し、その取消しボタン(復元前の世代)を作る。
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: BuildingType::Mine });
    app.update();
    // 選択対象を与えて建設パネルを作る(選択は地図側。ここでは SelectedView を直接設定)。
    app.world_mut().resource_mut::<strategy_game::hexmap::economy_view::SelectedView>().0 =
        strategy_game::hexmap::economy_view::SelectionTarget::State {
            state: MapStateId(20),
            province: strategy_game::world::ProvinceId(1),
            in_economy: true,
            countries: vec![1],
        };
    app.update();
    let old_cancel = find::<CancelButton>(&mut app);
    assert_eq!(old_cancel.len(), 1);
    let old_id = app.world().get::<CancelButton>(old_cancel[0]).unwrap().order;
    assert_eq!(app.world().get::<CancelButton>(old_cancel[0]).unwrap().epoch, 0);
    // 事前確認 → 復元(注文は無かった状態に戻る。注文連番も戻る)。
    send(&mut app, SessionCommand::InspectSave { name: "e".into() });
    app.update();
    send(&mut app, SessionCommand::LoadSave { actor: Some(1), name: "e".into() });
    app.update();
    app.update();
    assert_eq!(snap_app(&app), saved);
    assert!(app.world().get_entity(old_cancel[0]).is_err(), "the pre-restore rows are rebuilt");
    assert!(find::<CancelButton>(&mut app).is_empty());
    // 新しい注文は、復元で戻った連番のため、復元前と同じ識別子を得る。
    send(&mut app, SessionCommand::Build { actor: 1, state: MapStateId(20), building: BuildingType::Farm });
    app.update();
    app.update();
    let new_ids: Vec<_> = app.world().resource::<SessionRes>().session.country(1).unwrap().construction_queue.iter().map(|o| o.id).collect();
    assert_eq!(new_ids, vec![old_id], "the order counter came back, so the id is reused by a different order");
    // 復元前の世代のボタン(古い画面が残っていた場合を再現)を押しても、コマンドは発行されない。
    let stale = app
        .world_mut()
        .spawn((CancelButton { actor: 1, order: old_id, epoch: 0 }, Button, Interaction::None))
        .id();
    press_entity(&mut app, stale);
    app.update();
    let queue_now: Vec<_> = app.world().resource::<SessionRes>().session.country(1).unwrap().construction_queue.iter().map(|o| o.id).collect();
    assert_eq!(queue_now, vec![old_id], "the new order with the same id was not cancelled by the old button");
    let stale_build = app
        .world_mut()
        .spawn((
            BuildButton { actor: 1, state: MapStateId(20), building: BuildingType::Mine, epoch: 0 },
            Button,
            Interaction::None,
        ))
        .id();
    press_entity(&mut app, stale_build);
    let n = app.world().resource::<SessionRes>().session.country(1).unwrap().construction_queue.len();
    assert_eq!(n, 1, "an old build button does not order anything either");
    // 新しい世代のボタンは使える。
    let fresh = find::<CancelButton>(&mut app);
    let epochs: Vec<u64> = fresh.iter().map(|e| app.world().get::<CancelButton>(*e).unwrap().epoch).collect();
    assert!(epochs.contains(&1) && epochs.contains(&0) && fresh.len() == 2, "real row (epoch 1) + injected stale button: {epochs:?}");
}

#[test]
fn a_failed_restore_through_the_screen_keeps_state_actor_selection_and_other_results() {
    let t = Tmp::new("fail");
    let mut app = ui_app(t.loc(), session());
    set_actor(&mut app, 2);
    send(&mut app, SessionCommand::AdvanceDays(4));
    app.update();
    send(&mut app, save_cmd(Some(2), "ok", false));
    app.update();
    fs::write(t.file("broken"), "(kind: \"map_economy_state\", format_version: 2, trunc").unwrap();
    tamper_item(&t, "ok", "rejected", "grid");
    send(&mut app, SessionCommand::RefreshSaves);
    app.update();
    let before = snap_app(&app);
    let sel = app.world().resource::<HexSelection>().0;
    // 壊れたファイルの行は選択できない(ボタンが無い)。
    let rows: Vec<String> =
        find::<SaveRowButton>(&mut app).iter().map(|e| app.world().get::<SaveRowButton>(*e).unwrap().name.clone()).collect();
    assert!(!rows.contains(&"broken".to_string()) && rows.contains(&"ok".to_string()) && rows.contains(&"rejected".to_string()), "{rows:?}");
    let texts = all_texts(&mut app);
    assert!(texts.contains("破損") && texts.contains("選択できません"), "{texts}");
    // 拒否される保存: 確認で拒否と表示され、復元ボタンは無い。強制的に送っても復元されない。
    send(&mut app, SessionCommand::InspectSave { name: "rejected".into() });
    app.update();
    assert!(all_texts(&mut app).contains("復元できません(静的データが保存時と一致しません)"));
    assert!(find::<LoadSaveButton>(&mut app).is_empty(), "a rejected save offers no restore button");
    send(&mut app, SessionCommand::LoadSave { actor: Some(2), name: "rejected".into() });
    app.update();
    assert_eq!(snap_app(&app), before);
    assert_eq!(app.world().resource::<SessionUi>().actor, Some(2));
    assert_eq!(app.world().resource::<HexSelection>().0, sel);
    // 失敗しても操作結果は更新される(状態は変わらない)。
    assert!(log_text(&app).contains("復元できません"));
}

#[test]
fn a_long_list_keeps_every_row_in_name_order() {
    let t = Tmp::new("long");
    let mut app = ui_app(t.loc(), session());
    set_actor(&mut app, 1);
    for i in (0..25).rev() {
        send(&mut app, save_cmd(Some(1), &format!("slot_{i:02}"), false));
    }
    app.update();
    app.update();
    let rows: Vec<String> =
        find::<SaveRowButton>(&mut app).iter().map(|e| app.world().get::<SaveRowButton>(*e).unwrap().name.clone()).collect();
    assert_eq!(rows.len(), 25);
    let mut sorted = rows.clone();
    sorted.sort();
    let l: Vec<String> = app.world().resource::<SaveUi>().list.as_ref().unwrap().entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(l, sorted, "the list is in stable name order");
    assert_eq!(rows.len(), l.len());
}

#[test]
fn panel_hiding_and_detail_toggle_are_screen_only() {
    let t = Tmp::new("toggle");
    let mut app = ui_app(t.loc(), session());
    let before = snap_app(&app);
    send(&mut app, SessionCommand::ToggleSavePanel);
    app.update();
    assert!(!app.world().resource::<SaveUi>().show);
    let panel = find::<SavePanelRoot>(&mut app)[0];
    assert_eq!(app.world().get::<Node>(panel).unwrap().display, Display::None);
    send(&mut app, SessionCommand::ToggleSavePanel);
    send(&mut app, SessionCommand::ToggleSaveDetail);
    app.update();
    assert!(app.world().resource::<SaveUi>().show && app.world().resource::<SaveUi>().show_detail);
    assert_eq!(snap_app(&app), before);
    assert!(!t.dir().exists());
}
