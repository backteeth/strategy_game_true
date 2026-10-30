//! P32-022 (S3): 経済セッションの保存・一覧・事前確認・復元のUI(独立バイナリ `map_economy_session` 用)。
//!
//! **保存・復元・確認・一覧は P32-016〜019 の共有API(`EconomySession::{save, inspect, load, list_saves}`、実体は
//! `economy::map_economy_file`)をそのまま呼ぶ。** 保存名の検証(`SaveLocation::file_path`)・保存先の制限・リンクの扱い・
//! 一時ファイル経由の原子的置換・互換性判定(項目別の静的識別情報)・復元候補の検証は共有側のもので、この画面には
//! UI専用のパーサー・互換性判定・名前検証・ファイル書込処理は無い(ここにあるのは表示用の文言と、結果の保持だけ)。
//! 対象は `saves/map_economy/<名前>.mapeco.ron`。旧ゲームの `SaveGameV1`(`saves/savegame_v1.ron`)とは別の経路で、
//! 共有API側が旧V1のスロットを拒否する。
//!
//! ## コマンド(既存の単一適用経路)
//! 保存・確認・復元・更新は [`SessionCommand`](super::economy_view::SessionCommand) の `SaveAs` / `InspectSave` / `LoadSave` /
//! `RefreshSaves`(+表示切替)で、`apply_session_commands` が他のコマンド(日付進行・発注・取消し・国家選択)と**同じ列で到着順に**
//! 適用する([`apply_file_command`])。保存名・対象・操作主体は**ボタンを押した時点の値**をコマンドが保持し、適用時に行番号や
//! 入力欄から取り直さない。保存と復元は、承認済みの方針どおり**操作主体を明示してから**だけ実行できる(一覧・確認は未選択でも可)。
//!
//! ## 日境界
//! 経済セッションは1日の処理が完了した状態でしか外から見えない(`advance_day` は同期的に最後まで進む)ので、保存は常に
//! 「最後に処理を完了した日」の状態を対象にする。複数日進行の途中は他のコマンドが割り込めない。
//!
//! ## 復元
//! 復元は「一覧 → 行を選ぶと事前確認(共有 `inspect`)→ 内容の表示 → 復元実行(共有 `load`)」。`load` は確認結果を使わず
//! その時点のファイルと静的入力で最初から全検証し、失敗すれば経済状態を変えない。復元の前提として、事前確認が現在の状態に
//! 対して有効(同じ保存名・確認後に経済状態が変わっていない・互換または警告付き互換)であることも画面側で求める。
//! 成功時は、その時点で受け付けた**残りのコマンドを破棄**し(古い発注・取消し・日付進行が復元後の状態へ適用されない)、
//! 操作結果欄を整理し、建設候補・キューは共有セッション(正本)から再導出される。カメラとセル選択は触らない
//! (選択対象は毎フレーム復元後の正本から導出)。操作主体は現在の選択を再検証し、無効なら未選択にする(先頭国家へは置き換えない)。
//!
//! ## 入力
//! 保存名の入力欄にフォーカスがある間([`save_text_input`])、文字・編集キーは入力欄だけが受け、地図のショートカット
//! (表示モード・境界線・Home・Esc での終了など)へは渡さない(`ButtonInput<KeyCode>` の押下を打ち消す)。`Esc` はフォーカスを外す。
//! パネル全体が `Button` なので、クリック・ドラッグ・ホイールは既存の `camera_input` の抑止により地図へ渡らない。

use super::data::HexMap;
use super::economy_view::{
    SessionRes, SessionSet, SessionUi, SessionCommand, actor_error_text, date_text, fmt_date,
};
use super::view::{HexPreviewEntity, HexPreviewSet, HexPreviewState, LEFT_PANEL_W, button_bundle, label};
use crate::economy::map_economy_file::{
    DetectedKind, DirectoryStatus, EntryStatus, FileError, InspectReport, SaveEntry, SaveList, SaveLocation,
};
use crate::economy::map_session::EconomySession;
use crate::economy::map_state::snapshot::SnapshotDate;
use crate::economy::map_state::static_identity::{Finding, ItemKey, Structural, Verdict};
use crate::world::WorldModel;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

/// 保存名の入力欄に入力できる最大文字数(共有仕様の上限64を超えた分は理由つきで拒否表示するため、少し余裕を持たせる)。
pub const NAME_INPUT_MAX_CHARS: usize = 80;
const PANEL_X: f32 = LEFT_PANEL_W + 8.0 + 360.0 + 8.0;
const PANEL_W: f32 = 400.0;

// ---------------------------------------------------------------- 保存先と画面状態

/// 保存先(`<base>/map_economy/`)。既定は `saves`。テストや検証は一時ディレクトリを `insert_resource` で与える。
#[derive(Resource, Debug, Clone)]
pub struct SaveLocationRes(pub SaveLocation);

impl Default for SaveLocationRes {
    fn default() -> Self {
        SaveLocationRes(SaveLocation::new("saves"))
    }
}

/// 事前確認の結果(**確認した時点**のもの。復元の許可ではない)。
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub name: String,
    /// 確認した時点の経済状態の世代([`SaveUi::state_gen`])。違えば無効。
    pub gen_at: u64,
    /// 確認した時点の一覧上の状態(一覧の更新でこれと違えば無効)。
    pub entry: Option<EntryStatus>,
    pub result: Result<InspectReport, FileError>,
}

/// 保存・復元パネルの画面状態(経済値は持たない)。
#[derive(Resource, Debug)]
pub struct SaveUi {
    pub show: bool,
    /// 保存名の入力欄の内容と、フォーカス。
    pub name: String,
    pub focus: bool,
    /// 直近に取得した一覧(`None` = まだ取得していない)。
    pub list: Option<SaveList>,
    /// 選択中の保存名(**名前**で識別し、行番号は使わない)。
    pub selected: Option<String>,
    pub preview: Option<Preview>,
    pub show_detail: bool,
    /// 経済状態を変えた操作(日付進行・発注・取消し・復元)のたびに増える世代。事前確認の有効性の判定に使う。
    pub state_gen: u64,
    /// 復元に成功するたびに増える世代。建設・取消しボタンが保持し、復元前の古いボタンの操作を発行させない。
    pub restore_epoch: u64,
    /// 一覧・確認の表示を作り直す合図。
    pub rev: u64,
}

impl Default for SaveUi {
    fn default() -> Self {
        SaveUi {
            show: true,
            name: String::new(),
            focus: false,
            list: None,
            selected: None,
            preview: None,
            show_detail: false,
            state_gen: 0,
            restore_epoch: 0,
            rev: 0,
        }
    }
}

impl SaveUi {
    pub fn touch(&mut self) {
        self.rev += 1;
    }

    /// 経済状態が変わった(日付進行・発注・取消し・復元)。事前確認は現在の結果ではなくなるので捨てる。
    pub fn state_changed(&mut self) {
        self.state_gen += 1;
        self.preview = None;
        self.touch();
    }

    /// 新しい一覧を取り込む。選択中の保存が無くなれば選択と確認を無効にし、確認後に一覧上の状態が変わっていれば確認を無効にする。
    pub fn set_list(&mut self, list: SaveList) {
        if let Some(sel) = self.selected.clone() {
            match list.entries.iter().find(|e| e.name == sel) {
                None => {
                    self.selected = None;
                    self.preview = None;
                }
                Some(e) => {
                    if self.preview.as_ref().is_some_and(|p| p.name == sel && p.entry.as_ref() != Some(&e.status)) {
                        self.preview = None;
                    }
                }
            }
        }
        self.list = Some(list);
        self.touch();
    }

    /// 現在有効な事前確認(同じ保存名・確認後に経済状態が変わっていない)。
    pub fn fresh_preview(&self, name: &str) -> Option<&Preview> {
        self.preview.as_ref().filter(|p| p.name == name && p.gen_at == self.state_gen)
    }
}

// ---------------------------------------------------------------- 保存名の状態(表示用。判定は共有 `SaveLocation::file_path`)

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameState {
    Empty,
    /// 共有仕様で使えない名前(理由は共有APIのエラー)。
    Invalid(String),
    /// 同名の保存は無い。
    New,
    /// 同じ綴りの保存があり、保存すると**それを置換する**。
    Replace,
    /// 大文字小文字だけ違う保存がある(別ファイルへ黙って保存しない・置換でもない)。
    CaseConflict(String),
}

pub fn name_state(name: &str, list: Option<&SaveList>, location: &SaveLocation) -> NameState {
    if name.is_empty() {
        return NameState::Empty;
    }
    if let Err(e) = location.file_path(name) {
        return NameState::Invalid(file_error_text(&e));
    }
    let Some(list) = list else { return NameState::New };
    if list.entries.iter().any(|e| e.name == name) {
        return NameState::Replace;
    }
    match list.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
        Some(e) => NameState::CaseConflict(e.name.clone()),
        None => NameState::New,
    }
}

// ---------------------------------------------------------------- 文言(表示専用)

/// 共有APIの失敗を利用者向けの日本語にする(原因は共有APIのものをそのまま分類し、置き換えない)。
pub fn file_error_text(e: &FileError) -> String {
    match e {
        FileError::InvalidName(_) => {
            "保存名が使えません(英数字・_・- のみ、1〜64文字、予約名は不可。パスや拡張子は入力しません)".to_string()
        }
        FileError::LocationRefused(_) => {
            "保存先が許可されていません(saves/map_economy/ の中の .mapeco.ron だけを扱います。旧V1のスロットは使えません)"
                .to_string()
        }
        FileError::Symlink(_) => "保存先またはファイルがシンボリックリンク等のため拒否しました".to_string(),
        FileError::NotFound(_) => "ファイルが見つかりません".to_string(),
        FileError::Io { op, message, .. } => format!("ファイル操作に失敗しました({op}: {message})"),
        FileError::TooLarge { bytes } => format!("ファイルが大きすぎます({bytes} バイト)"),
        FileError::NotThisKind(DetectedKind::LegacyV1 { .. }) => {
            "旧ゲームのセーブ(V1)で、この画面の形式ではありません。読込も上書きもしません".to_string()
        }
        FileError::NotThisKind(DetectedKind::Other(k)) => format!("別の種類のファイルです(kind \"{k}\")"),
        FileError::UnsupportedVersion { found: 1 } => {
            "形式バージョン1は静的データの照合情報がなく確認できないため、読み込めません(変換も上書きもしません)".to_string()
        }
        FileError::UnsupportedVersion { found } => format!("非対応の形式バージョン {found} です"),
        FileError::Identity(err) => format!("静的データの照合情報を作れません: {err}"),
        FileError::Incompatible(v) => format!("静的データが保存時と一致しないため復元できません({})", rejection_summary(v)),
        FileError::Malformed(m) => format!("ファイルが壊れているか、形式が正しくありません({m})"),
        FileError::Serialize(m) => format!("保存内容を作れませんでした({m})"),
        FileError::OverwriteRefused { .. } => {
            "同名のファイルが保存形式として識別できないため、上書きしませんでした".to_string()
        }
        FileError::Restore(r) => format!("保存内容を現在の状態へ復元できません({r})"),
    }
}

/// 静的入力の項目名の表示(内部名は詳細にだけ出す)。
pub fn item_label(k: ItemKey) -> &'static str {
    match k {
        ItemKey::Grid => "地図の格子",
        ItemKey::CellCoordinates => "セル座標",
        ItemKey::LandSet => "陸地の範囲",
        ItemKey::WaterKinds => "水域の種類",
        ItemKey::LandTerrain => "地形",
        ItemKey::Climate => "気候",
        ItemKey::Vegetation => "植生",
        ItemKey::Rivers => "河川",
        ItemKey::ProvinceTable => "プロヴィンス表",
        ItemKey::StateMembership => "ステートの所属",
        ItemKey::StatePresentation => "ステートの表示情報",
        ItemKey::StateConstraints => "ステートの制約",
        ItemKey::ScenarioStructure => "シナリオ構造",
        ItemKey::StateEconomyStatic => "ステート経済の静的値",
        ItemKey::ResourceDeposits => "資源の鉱床",
        ItemKey::ResourceSetup => "資源設定",
        ItemKey::MilitaryInputs => "軍事の入力",
        ItemKey::BuildingComputation => "建物の計算定義",
        ItemKey::BuildingPresentation => "建物の表示情報",
        ItemKey::LocationFacilities => "位置施設",
        ItemKey::EconomyRules => "経済ルール版",
    }
}

fn rejection_summary(v: &Verdict) -> String {
    let mut parts: Vec<String> = v.structural.iter().map(structural_text).collect();
    parts.extend(v.rejections.iter().map(|f| format!("{}が異なる", item_label(f.key))));
    parts.join("、")
}

fn structural_text(s: &Structural) -> String {
    match s {
        Structural::UnsupportedScheme { .. } | Structural::UnsupportedSchemeVersion { .. } => {
            "照合方式が非対応".to_string()
        }
        Structural::MissingItem(k) => format!("{}の照合情報が欠けている", item_label(*k)),
        Structural::UnknownItem(_) => "未知の照合項目がある".to_string(),
        Structural::DuplicateItem(_) => "照合項目が重複している".to_string(),
        Structural::MalformedDigest { .. } => "照合値の形式が不正".to_string(),
    }
}

pub fn snapshot_date_text(d: SnapshotDate) -> String {
    format!("{:04}/{:02}/{:02}", d.year, d.month, d.day)
}

fn bytes_text(b: usize) -> String {
    if b >= 1024 { format!("{:.1} KB", b as f64 / 1024.0) } else { format!("{b} B") }
}

/// 一覧の1行の文言と、選択できるか。選択できない行は理由を含む。
pub fn row_text(e: &SaveEntry) -> (String, bool) {
    match &e.status {
        EntryStatus::Valid { date, bytes } => (
            format!(
                "{}  処理済み {}  {}\n形式は正常(静的データとの互換性は選択して事前確認)",
                e.name,
                snapshot_date_text(*date),
                bytes_text(*bytes)
            ),
            true,
        ),
        EntryStatus::Unusable(err) => {
            let kind = match err {
                FileError::Malformed(_) | FileError::TooLarge { .. } => "破損",
                FileError::NotThisKind(_) | FileError::UnsupportedVersion { .. } => "非対応の形式",
                _ => "使用不可",
            };
            (format!("{}  [{kind}・選択できません]\n{}", e.name, file_error_text(err)), false)
        }
    }
}

// ---------------------------------------------------------------- 事前確認の表示モデル(純粋関数)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewClass {
    /// 完全一致で復元できる。
    Ok,
    /// 警告つきで復元できる(警告を確認できる状態から実行する)。
    Warn,
    /// 復元できない(拒否・確認不能)。
    Reject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewModel {
    pub class: PreviewClass,
    pub title: String,
    pub lines: Vec<String>,
    /// 折りたたみ(内部名・照合値の先頭・根拠)。
    pub detail: Vec<String>,
}

fn finding_detail(f: &Finding) -> String {
    let cut = |s: &str| s.chars().take(12).collect::<String>();
    format!(
        "{}: 保存時 {}… / 現在 {}… — {} ({})",
        f.key.name(),
        cut(&f.saved),
        cut(&f.current),
        f.rationale,
        f.impact
    )
}

pub fn preview_model(p: &Preview) -> PreviewModel {
    match &p.result {
        Err(e) => PreviewModel {
            class: PreviewClass::Reject,
            title: format!("「{}」は確認できません(復元できません)", p.name),
            lines: vec![file_error_text(e)],
            detail: vec![format!("{e}")],
        },
        Ok(r) => {
            let mut lines = vec![
                format!("形式バージョン {}(対応)", r.format_version),
                format!("処理済み日付: {}", snapshot_date_text(r.date)),
                format!(
                    "内容: ステート {} / 国家 {} / 建設キュー {} 件",
                    r.summary.states, r.summary.countries, r.summary.queued_orders
                ),
            ];
            let mut detail: Vec<String> = Vec::new();
            for s in &r.verdict.structural {
                lines.push(format!("拒否: {}", structural_text(s)));
                detail.push(format!("{s}"));
            }
            for f in &r.verdict.rejections {
                lines.push(format!("拒否: {}が保存時と異なります", item_label(f.key)));
                detail.push(finding_detail(f));
            }
            for f in &r.verdict.warnings {
                lines.push(format!("警告: {}が保存時と異なります(復元は可能)", item_label(f.key)));
                detail.push(finding_detail(f));
            }
            let class;
            let title;
            if !r.verdict.is_compatible() {
                class = PreviewClass::Reject;
                title = format!("「{}」は復元できません(静的データが保存時と一致しません)", p.name);
            } else if let Some(Err(e)) = &r.restorable {
                class = PreviewClass::Reject;
                title = format!("「{}」は復元できません(保存内容の検証に失敗)", p.name);
                lines.push(format!("拒否: {e}"));
            } else if !r.verdict.warnings.is_empty() {
                class = PreviewClass::Warn;
                title = format!("「{}」は警告つきで復元できます", p.name);
            } else {
                class = PreviewClass::Ok;
                title = format!("「{}」は復元できます(静的データは完全一致)", p.name);
            }
            detail.push(format!("{}項目一致 / 警告 {} / 拒否 {}", r.verdict.matched.len(), r.verdict.warnings.len(), r.verdict.rejections.len()));
            PreviewModel { class, title, lines, detail }
        }
    }
}

// ---------------------------------------------------------------- コマンドの適用(共有APIを呼ぶだけ)

/// 保存・復元に必要な読み取り専用の文脈。地図・ワールドが無ければ保存・確認・復元はできない。
pub struct FileCtx<'a> {
    pub map: Option<&'a HexMap>,
    pub world: Option<&'a WorldModel>,
    pub location: &'a SaveLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResult {
    /// 結果欄へ出す文言(画面だけの切替は `None`)。
    pub text: Option<(bool, String)>,
    /// 復元が成功した(呼び出し側は、その時点で受け付けた残りのコマンドを破棄する)。
    pub restored: bool,
}

impl FileResult {
    fn ok(t: String) -> Self {
        FileResult { text: Some((true, t)), restored: false }
    }
    fn fail(t: String) -> Self {
        FileResult { text: Some((false, t)), restored: false }
    }
    fn silent() -> Self {
        FileResult { text: None, restored: false }
    }
}

pub fn is_file_command(c: &SessionCommand) -> bool {
    matches!(
        c,
        SessionCommand::SaveAs { .. }
            | SessionCommand::InspectSave { .. }
            | SessionCommand::LoadSave { .. }
            | SessionCommand::RefreshSaves
            | SessionCommand::ToggleSavePanel
            | SessionCommand::ToggleSaveDetail
    )
}

fn actor_gate(session: &EconomySession, current: Option<u32>, commanded: Option<u32>, what: &str) -> Result<(), FileResult> {
    if commanded != current {
        let show = |a: Option<u32>| a.map_or("未選択".to_string(), |l| format!("国家{l}"));
        return Err(FileResult::fail(format!(
            "{what}を実行しませんでした: 操作国家が変わっています(操作時: {} / 現在: {})",
            show(commanded),
            show(current)
        )));
    }
    session
        .resolve_actor(current)
        .map(|_| ())
        .map_err(|e| FileResult::fail(format!("{what}できません: {}", actor_error_text(&e))))
}

fn refresh(save: &mut SaveUi, location: &SaveLocation) {
    save.set_list(EconomySession::list_saves(location));
}

/// 保存・一覧・確認・復元のコマンドを適用する(`apply_session_commands` がコマンド列の中で呼ぶ)。
/// 失敗・拒否では `session`・`actor` を変更しない(結果の文言と画面状態 `save` だけが変わりうる)。
pub fn apply_file_command(
    session: &mut EconomySession,
    actor: &mut Option<u32>,
    save: &mut SaveUi,
    ctx: &FileCtx<'_>,
    cmd: SessionCommand,
) -> FileResult {
    match cmd {
        SessionCommand::ToggleSavePanel => {
            save.show = !save.show;
            save.touch();
            FileResult::silent()
        }
        SessionCommand::ToggleSaveDetail => {
            save.show_detail = !save.show_detail;
            save.touch();
            FileResult::silent()
        }
        SessionCommand::RefreshSaves => {
            refresh(save, ctx.location);
            let n = save.list.as_ref().map_or(0, |l| l.entries.len());
            FileResult::ok(format!("保存一覧を更新しました({n} 件)"))
        }
        SessionCommand::SaveAs { actor: commanded, name, replace } => {
            if let Err(r) = actor_gate(session, *actor, commanded, "保存") {
                return r;
            }
            let (Some(map), Some(world)) = (ctx.map, ctx.world) else {
                return FileResult::fail("保存できません: 地図またはワールド設定がありません".to_string());
            };
            // 名前の検証は共有仕様(`SaveLocation::file_path`)。
            if let Err(e) = ctx.location.file_path(&name) {
                return FileResult::fail(format!("保存できません: {}", file_error_text(&e)));
            }
            // 置換は明示された場合だけ。適用時点の一覧で、同じ綴りの保存があることを確かめる。
            let fresh = EconomySession::list_saves(ctx.location);
            let exact = fresh.entries.iter().any(|e| e.name == name);
            let similar = fresh.entries.iter().find(|e| e.name != name && e.name.eq_ignore_ascii_case(&name));
            if let Some(s) = similar {
                return FileResult::fail(format!(
                    "保存できません: 既存の保存「{}」と大文字小文字だけが違います(別ファイルへ黙って保存しません)",
                    s.name
                ));
            }
            if exact && !replace {
                return FileResult::fail(format!(
                    "保存できません: 「{name}」はすでにあります(置換するには「置換して保存」を押してください)"
                ));
            }
            match session.save(map, world, ctx.location, &name) {
                Ok(r) => {
                    // 保存したファイルが変わったので、同名の事前確認は古い。一覧も取り直す(経済状態は変えない)。
                    if save.preview.as_ref().is_some_and(|p| p.name == name) {
                        save.preview = None;
                    }
                    refresh(save, ctx.location);
                    let sc = session.scenario();
                    let when = if session.sim.date == sc.start { " (開始日のまま・未処理)" } else { "" };
                    FileResult::ok(format!(
                        "保存しました: 「{name}」{}  処理済み日付 {}{when}",
                        if r.replaced_existing { "(既存を置換)" } else { "(新規)" },
                        snapshot_date_text(r.date)
                    ))
                }
                Err(e) => FileResult::fail(format!("保存に失敗しました(既存のファイルは変更していません): {}", file_error_text(&e))),
            }
        }
        SessionCommand::InspectSave { name } => {
            let (Some(map), Some(world)) = (ctx.map, ctx.world) else {
                return FileResult::fail("確認できません: 地図またはワールド設定がありません".to_string());
            };
            // 共有 `inspect`(復元せず、ファイルもセッションも変えない)。一覧も取り直して、確認時点の状態を記録する。
            let result = session.inspect(map, world, ctx.location, &name);
            let fresh = EconomySession::list_saves(ctx.location);
            let entry = fresh.entries.iter().find(|e| e.name == name).map(|e| e.status.clone());
            save.selected = Some(name.clone());
            let preview = Preview { name: name.clone(), gen_at: save.state_gen, entry, result };
            let model = preview_model(&preview);
            save.preview = Some(preview);
            save.set_list(fresh);
            // set_list は選択中の保存が一覧に無ければ選択を外す(ファイルが消えている場合)。
            let text = format!("事前確認: {}", model.title);
            if model.class == PreviewClass::Reject { FileResult::fail(text) } else { FileResult::ok(text) }
        }
        SessionCommand::LoadSave { actor: commanded, name } => {
            if let Err(r) = actor_gate(session, *actor, commanded, "復元") {
                return r;
            }
            let (Some(map), Some(world)) = (ctx.map, ctx.world) else {
                return FileResult::fail("復元できません: 地図またはワールド設定がありません".to_string());
            };
            // 画面側の前提: この保存を、現在の状態に対して事前確認済みで、復元可能または警告つき可能であること。
            let Some(p) = save.fresh_preview(&name) else {
                return FileResult::fail(format!(
                    "復元できません: 「{name}」の事前確認が現在の状態に対して行われていません(保存を選んで確認してください)"
                ));
            };
            if preview_model(p).class == PreviewClass::Reject {
                return FileResult::fail(format!("復元できません: 「{name}」は事前確認で拒否されています"));
            }
            // 共有 `load`: 確認結果は使わず、その時点のファイルと静的入力で全検証をやり直す。失敗時は `session` を変更しない。
            match session.load(map, world, ctx.location, &name) {
                Ok(r) => {
                    save.restore_epoch += 1;
                    save.state_changed();
                    refresh(save, ctx.location);
                    let mut t = format!(
                        "復元しました: 「{name}」  処理済み日付 {}  次に処理する日 {}(時間は停止中。自動では進みません)",
                        snapshot_date_text(r.date),
                        fmt_date(session.sim.date.next().0)
                    );
                    if !r.verdict.warnings.is_empty() {
                        let w: Vec<&str> = r.verdict.warnings.iter().map(|f| item_label(f.key)).collect();
                        t.push_str(&format!("\n警告つきで復元: {}が保存時と異なります", w.join("、")));
                    }
                    // 操作主体は現在の選択を再検証する(先頭国家へは置き換えない)。
                    if session.resolve_actor(*actor).is_err() {
                        *actor = None;
                        t.push_str("\n操作国家の選択は復元後の状態では無効なため、未選択にしました");
                    }
                    FileResult { text: Some((true, t)), restored: true }
                }
                Err(e) => {
                    // 確認結果と実際のファイルが食い違った可能性があるので、確認を捨てて一覧を取り直す。
                    save.preview = None;
                    refresh(save, ctx.location);
                    FileResult::fail(format!("復元に失敗しました(経済状態は変更していません): {}", file_error_text(&e)))
                }
            }
        }
        _ => FileResult::fail("保存・復元のコマンドではありません".to_string()),
    }
}

// ---------------------------------------------------------------- 画面

#[derive(Component)]
pub struct SavePanelRoot;
#[derive(Component)]
pub struct SaveHeaderText;
#[derive(Component)]
pub struct SaveNameField;
#[derive(Component)]
pub struct SaveNameText;
#[derive(Component)]
pub struct SaveNoteText;
#[derive(Component)]
pub struct SaveSubmitButton;
#[derive(Component)]
pub struct SaveSubmitLabel;
#[derive(Component)]
pub struct RefreshSavesButton;
#[derive(Component)]
pub struct ToggleSavePanelButton;
#[derive(Component)]
pub struct ToggleSaveDetailButton;
#[derive(Component)]
pub struct SaveListRoot;
/// 保存一覧の行(**保存名**で対象を指す。行番号は使わない)。
#[derive(Component, Debug, Clone)]
pub struct SaveRowButton {
    pub name: String,
}
/// 復元ボタン(確認した保存名を保持する)。
#[derive(Component, Debug, Clone)]
pub struct LoadSaveButton {
    pub name: String,
}

pub fn spawn_save_panel(commands: &mut Commands) {
    let bg = BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.95));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PANEL_X),
                top: Val::Px(112.0),
                bottom: Val::Px(150.0),
                width: Val::Px(PANEL_W),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(6.0),
                ..default()
            },
            Button,
            bg,
            SavePanelRoot,
            HexPreviewEntity,
        ))
        .with_children(|p| {
            p.spawn(label("保存・復元(検証用 経済セッション)", 14.0));
            p.spawn((SaveHeaderText, label("", 12.0)));
            p.spawn(label("保存名(英数字・_・-、1〜64文字):", 12.0));
            // 入力欄は Button(Interaction を発行して地図への入力を止め、クリックでフォーカスする)。
            p.spawn((
                SaveNameField,
                Button,
                Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)), min_height: Val::Px(28.0), ..default() },
                BackgroundColor(Color::srgb(0.13, 0.14, 0.18)),
            ))
            .with_children(|f| {
                f.spawn((SaveNameText, label("", 13.0)));
            });
            p.spawn((SaveNoteText, label("", 12.0)));
            p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(6.0), flex_wrap: FlexWrap::Wrap, ..default() })
                .with_children(|r| {
                    r.spawn((
                        SaveSubmitButton,
                        Button,
                        Node { padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)), ..default() },
                        BackgroundColor(Color::srgb(0.22, 0.34, 0.27)),
                    ))
                    .with_children(|b| {
                        b.spawn((SaveSubmitLabel, label("保存", 13.0)));
                    });
                    r.spawn(button_bundle(RefreshSavesButton, "一覧を更新"));
                });
            crate::ui::scroll::spawn_scrollable_body(p, Val::Px(4.0), |body| {
                body.spawn((SaveListRoot, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), ..default() }));
            });
        });
}

fn dim_label(text: String, size: f32) -> impl Bundle {
    (Text::new(text), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(Color::srgb(0.62, 0.62, 0.66)))
}

fn colored_label(text: String, size: f32, c: Color) -> impl Bundle {
    (Text::new(text), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(c))
}

fn spawn_save_rows(list: &mut ChildSpawnerCommands, save: &SaveUi) {
    let box_bg = Color::srgba(0.16, 0.17, 0.2, 0.9);
    // ---- 事前確認
    list.spawn(label("事前確認", 13.0));
    match save.preview.as_ref().filter(|p| p.gen_at == save.state_gen) {
        None => {
            list.spawn(dim_label("(保存を選ぶと、復元せずに内容と互換性を確認します)".to_string(), 12.0));
        }
        Some(p) => {
            let m = preview_model(p);
            let color = match m.class {
                PreviewClass::Ok => Color::srgb(0.6, 0.9, 0.65),
                PreviewClass::Warn => Color::srgb(0.95, 0.85, 0.45),
                PreviewClass::Reject => Color::srgb(0.95, 0.55, 0.5),
            };
            list.spawn((Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), padding: UiRect::all(Val::Px(6.0)), ..default() }, BackgroundColor(box_bg)))
                .with_children(|b| {
                    b.spawn(colored_label(m.title.clone(), 12.0, color));
                    for l in &m.lines {
                        b.spawn(label(l.clone(), 12.0));
                    }
                    if m.class != PreviewClass::Reject {
                        let text = if m.class == PreviewClass::Warn {
                            format!("警告を確認して「{}」を復元", p.name)
                        } else {
                            format!("「{}」を復元", p.name)
                        };
                        b.spawn((
                            LoadSaveButton { name: p.name.clone() },
                            Button,
                            Node { padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)), ..default() },
                            BackgroundColor(Color::srgb(0.3, 0.3, 0.5)),
                        ))
                        .with_children(|bb| {
                            bb.spawn(label(text, 13.0));
                        });
                        b.spawn(dim_label("復元すると現在の経済状態(日付・国庫・建設キュー)は保存時点のものに置き換わります。".to_string(), 11.0));
                    }
                    b.spawn(button_bundle(ToggleSaveDetailButton, if save.show_detail { "詳細を隠す" } else { "詳細を表示" }));
                    if save.show_detail {
                        for d in &m.detail {
                            b.spawn(dim_label(d.clone(), 11.0));
                        }
                    }
                });
        }
    }
    // ---- 一覧
    list.spawn(label("保存一覧(名前順)", 13.0));
    match &save.list {
        None => {
            list.spawn(dim_label("(まだ取得していません。「一覧を更新」を押してください)".to_string(), 12.0));
        }
        Some(l) => {
            match &l.directory {
                DirectoryStatus::Missing => {
                    list.spawn(dim_label("保存なし(保存先フォルダはまだありません。一覧の取得では作りません)".to_string(), 12.0));
                }
                DirectoryStatus::Refused(e) => {
                    list.spawn(colored_label(
                        format!("保存先フォルダに問題があります: {}", file_error_text(e)),
                        12.0,
                        Color::srgb(0.95, 0.55, 0.5),
                    ));
                }
                DirectoryStatus::Readable => {
                    if l.entries.is_empty() {
                        list.spawn(dim_label("保存なし".to_string(), 12.0));
                    }
                }
            }
            for e in &l.entries {
                let (text, selectable) = row_text(e);
                let mark = if save.selected.as_deref() == Some(e.name.as_str()) { "▶ " } else { "" };
                if selectable {
                    list.spawn((
                        SaveRowButton { name: e.name.clone() },
                        Button,
                        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() },
                        BackgroundColor(Color::srgb(0.22, 0.26, 0.34)),
                    ))
                    .with_children(|b| {
                        b.spawn(label(format!("{mark}{text}"), 12.0));
                    });
                } else {
                    list.spawn((Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() }, BackgroundColor(box_bg)))
                        .with_children(|b| {
                            b.spawn(dim_label(text, 12.0));
                        });
                }
            }
        }
    }
}

/// 一覧・確認の表示を作り直す(`SaveUi.rev` が変わったフレームだけ。経済処理は再実行しない)。
pub fn sync_save_panel(
    mut commands: Commands,
    save: Res<SaveUi>,
    list_root: Query<Entity, With<SaveListRoot>>,
    mut last: Local<Option<u64>>,
) {
    if *last == Some(save.rev) {
        return;
    }
    let Ok(root) = list_root.single() else { return };
    *last = Some(save.rev);
    commands.entity(root).despawn_related::<Children>().with_children(|l| spawn_save_rows(l, &save));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn sync_save_texts(
    session: Option<Res<SessionRes>>,
    ui: Res<SessionUi>,
    save: Res<SaveUi>,
    loc: Res<SaveLocationRes>,
    mut header: Query<&mut Text, (With<SaveHeaderText>, Without<SaveNameText>, Without<SaveNoteText>, Without<SaveSubmitLabel>)>,
    mut name: Query<&mut Text, (With<SaveNameText>, Without<SaveHeaderText>, Without<SaveNoteText>, Without<SaveSubmitLabel>)>,
    mut note: Query<&mut Text, (With<SaveNoteText>, Without<SaveHeaderText>, Without<SaveNameText>, Without<SaveSubmitLabel>)>,
    mut submit: Query<&mut Text, (With<SaveSubmitLabel>, Without<SaveHeaderText>, Without<SaveNameText>, Without<SaveNoteText>)>,
    mut field: Query<&mut BackgroundColor, With<SaveNameField>>,
) {
    if let (Ok(mut t), Some(s)) = (header.single_mut(), &session) {
        let sc = s.session.scenario();
        t.0 = format!(
            "対象: 検証用 経済セッション(シナリオ {})\n{}\n保存先: {}/<名前>.mapeco.ron\n(旧ゲームの saves/savegame_v1.ron とは別。上書きしません)",
            sc.label,
            date_text(sc.start, s.session.sim.date),
            loc.0.dir().display()
        );
    }
    let state = name_state(&save.name, save.list.as_ref(), &loc.0);
    if let Ok(mut t) = name.single_mut() {
        t.0 = if save.name.is_empty() && !save.focus {
            "(ここをクリックして保存名を入力)".to_string()
        } else {
            format!("{}{}", save.name, if save.focus { "|" } else { "" })
        };
    }
    if let Ok(mut bg) = field.single_mut() {
        bg.0 = if save.focus { Color::srgb(0.2, 0.24, 0.34) } else { Color::srgb(0.13, 0.14, 0.18) };
    }
    if let Ok(mut t) = note.single_mut() {
        t.0 = match &state {
            NameState::Empty => "保存名を入力してください".to_string(),
            NameState::Invalid(why) => format!("この名前は使えません: {why}"),
            NameState::New => format!("新規保存: {}.mapeco.ron", save.name),
            NameState::Replace => format!("同名の保存があります。保存すると「{}」を置換します", save.name),
            NameState::CaseConflict(other) => {
                format!("既存の保存「{other}」と大文字小文字だけが違います。置換するなら同じ綴りで、別名にするなら別の名前にしてください")
            }
        };
    }
    if let Ok(mut t) = submit.single_mut() {
        t.0 = match &state {
            NameState::Replace => format!("「{}」を置換して保存", save.name),
            _ => "新規保存".to_string(),
        };
        if ui.actor.is_none() {
            t.0.push_str("(要:操作国家の選択)");
        }
    }
}

pub fn sync_save_panel_visibility(save: Res<SaveUi>, mut panel: Query<&mut Node, With<SavePanelRoot>>) {
    if !save.is_changed() {
        return;
    }
    for mut n in &mut panel {
        n.display = if save.show { Display::Flex } else { Display::None };
    }
}

/// ボタンを押した瞬間に1回だけ、**その時点の値**でコマンドを発行する(状態は変えない)。
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn save_buttons(
    submit: Query<&Interaction, (Changed<Interaction>, With<SaveSubmitButton>)>,
    refresh: Query<&Interaction, (Changed<Interaction>, With<RefreshSavesButton>)>,
    toggle: Query<&Interaction, (Changed<Interaction>, With<ToggleSavePanelButton>)>,
    detail: Query<&Interaction, (Changed<Interaction>, With<ToggleSaveDetailButton>)>,
    rows: Query<(&Interaction, &SaveRowButton), Changed<Interaction>>,
    loads: Query<(&Interaction, &LoadSaveButton), Changed<Interaction>>,
    ui: Res<SessionUi>,
    save: Res<SaveUi>,
    loc: Res<SaveLocationRes>,
    mut out: MessageWriter<SessionCommand>,
) {
    for i in &submit {
        if *i == Interaction::Pressed {
            let replace = name_state(&save.name, save.list.as_ref(), &loc.0) == NameState::Replace;
            out.write(SessionCommand::SaveAs { actor: ui.actor, name: save.name.clone(), replace });
        }
    }
    for i in &refresh {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::RefreshSaves);
        }
    }
    for i in &toggle {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::ToggleSavePanel);
        }
    }
    for i in &detail {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::ToggleSaveDetail);
        }
    }
    for (i, b) in &rows {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::InspectSave { name: b.name.clone() });
        }
    }
    for (i, b) in &loads {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::LoadSave { actor: ui.actor, name: b.name.clone() });
        }
    }
}

/// 起動後に1回だけ一覧を取得する(読み取りのみ。保存先が無くても作らない)。
pub fn initial_refresh(session: Option<Res<SessionRes>>, mut done: Local<bool>, mut out: MessageWriter<SessionCommand>) {
    if !*done && session.is_some() {
        *done = true;
        out.write(SessionCommand::RefreshSaves);
    }
}

/// 保存名の入力。フォーカス中は文字・編集キーを入力欄だけが受け、地図のショートカットへ渡さない
/// (`HexPreviewSet::Main` より前に押下を打ち消す)。`Esc` はフォーカスを外す(終了しない)。
/// 画面だけの状態(経済には触れない)なので、経済コマンドの列には入れない。
#[allow(clippy::type_complexity)]
pub fn save_text_input(
    mut events: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    field: Query<&Interaction, With<SaveNameField>>,
    mut save: ResMut<SaveUi>,
) {
    let on_field = field.iter().any(|i| matches!(*i, Interaction::Pressed | Interaction::Hovered));
    if mouse.just_pressed(MouseButton::Left) {
        let want = on_field && save.show;
        if save.focus != want {
            save.focus = want;
        }
    }
    let focused = save.focus;
    let mut name = save.name.clone();
    let mut unfocus = false;
    for ev in events.read() {
        if !focused || ev.state != ButtonState::Pressed {
            continue;
        }
        match &ev.logical_key {
            Key::Backspace => {
                name.pop();
            }
            Key::Escape => unfocus = true,
            Key::Enter | Key::Tab => {}
            Key::Space => name.push(' '),
            Key::Character(s) => {
                for c in s.chars().filter(|c| !c.is_control()) {
                    if name.chars().count() < NAME_INPUT_MAX_CHARS {
                        name.push(c);
                    }
                }
            }
            _ => {}
        }
    }
    if focused {
        if name != save.name {
            save.name = name;
        }
        if unfocus {
            save.focus = false;
        }
        // 地図・終了などのショートカットへ渡さない。
        let pressed: Vec<KeyCode> = keys.get_just_pressed().copied().collect();
        for k in pressed {
            keys.clear_just_pressed(k);
        }
    }
}

/// 保存・復元パネルの更新を登録する。
pub fn add_save_panel_systems(app: &mut App) {
    app.add_systems(
        Update,
        save_text_input.before(HexPreviewSet::Main).run_if(in_state(HexPreviewState::Open)),
    )
    .add_systems(Update, (save_buttons, initial_refresh).in_set(SessionSet::Input))
    .add_systems(Update, (sync_save_panel, sync_save_texts, sync_save_panel_visibility).in_set(SessionSet::Sync));
}
