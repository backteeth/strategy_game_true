//! P32-020 (S1): 新マップ経済セッションの画面(独立バイナリ `map_economy_session` 用)。
//! 経済状態の正本は P32-019 の共有セッション([`EconomySession`](crate::economy::map_session::EconomySession)、
//! 中身は `MapEconomySimulation` 1つ)で、この画面はそれを**読んで表示し、コマンド経由で進める**だけ。
//! 国庫・人口・雇用・施設の別コピーや、日次・月次の再実装は無い。
//!
//! ## 範囲(S1)
//! 新マップの表示(既存の `HexPreviewPlugin`)、操作主体の明示的な選択、経済状態の表示、日単位の時間進行。
//! **含まない**: 建設注文・キャンセルのUI(S2)、保存・復元のUI(S3)、自動的な時間進行、旧ゲーム本体への統合。
//!
//! ## 構成
//! - [`SessionRes`]: 共有セッション(正本)。起動に失敗したら作らない([`SessionLoadError`] だけが残り、操作できない)。
//! - `SessionUi`: 明示的に選んだ操作主体(`Option<u32>`、既定は**未選択**)と、直近の操作結果。
//! - `SelectedView`: 選択セルから毎フレーム導出する対象(経済ステート・未割当・範囲外・水域…)。保持するだけで正本ではない。
//! - [`SessionCommand`]: UI操作はすべてこのコマンド(`Message`)にして、[`apply_command`] の**1つの経路で到着順に**適用する。
//!
//! ## コマンドの受付と適用
//! 1. ボタン(`Interaction` の `Changed`: 押した瞬間に1回だけ)が `SessionCommand` を発行する。状態は変えない。
//! 2. `apply_session_commands` が、そのフレームに到着した順に1つずつ [`apply_command`] で適用する。
//!    操作主体の検証は**適用の時点**(現在の `SessionUi.actor` に対して)に行う。
//! 3. `AdvanceDays(n)` は1コマンドとして `n`(1〜[`MAX_DAYS_PER_COMMAND`])回の `advance_day` を同期的に連続して行い、
//!    その間に他のコマンドや描画更新は入らない。`advance_day` は `Result` を返さない(失敗経路が無い)ので「途中までの進行」
//!    は起きない。プロセスが途中で異常終了すれば状態は失われる(巻き戻し・保存はしない。この画面にその保証は無い)。
//! 4. 拒否(未選択・AI国家・不正な日数)は状態を一切変えず、理由を結果欄に出す。
//!
//! 初回表示・表示モード変更・セル選択・国家選択・ウィンドウ再描画は日付を進めない(コマンドが無ければ何も進まない)。
//!
//! ## 日付
//! `sim.date` は「最後に処理を完了した日」。開始直後(`date == scenario.start`)は「未処理」と表示し、次に処理する日
//! (`date.next()`)を併記する。月次の値(雇用・物流・稼働率・収支)は、最初の月次処理までは**未計算**と表示し、0としては出さない。
//!
//! ## 入力
//! パネル・帯・ボタンはすべて `Button`(`Interaction` を発行)なので、既存の `camera_input` の抑止
//! (`ui_is_hovered`・`DragState.started_on_ui`)がそのまま効き、UI上のクリック・ドラッグ・スクロールは地図へ渡らない。
//! S3 で保存名の入力欄が加わった(`save_view`: フォーカス中は文字・編集キーを入力欄だけが受け、地図のショートカットへ渡さない)。
//! `Tab`(画面を閉じる=セッション破棄)は引き続き無効。`Esc` は(入力欄にフォーカスが無ければ)従来どおり終了で、保存していない状態は破棄される。
//!
//! ## S3(保存・復元)
//! 保存・確認・復元・一覧のコマンドも同じ列で適用する(`save_view::apply_file_command`)。復元が成功したら、その時点で受け付けた
//! 残りのコマンドは破棄する。

use super::data::HexMap;
use super::view::{
    HexPreviewEntity, HexPreviewSet, HexPreviewState, HexSelection, LEFT_PANEL_W, LoadedHexMap, RIGHT_PANEL_W,
    RightPanel, StatusText, button_bundle, enter_preview, label,
};
use super::build_view::{
    BuildButton, CancelButton, ToggleBuildPanelButton, add_build_panel_systems, spawn_build_panel,
};
use super::save_view::{
    FileCtx, SaveLocationRes, SaveUi, ToggleSavePanelButton, add_save_panel_systems, apply_file_command, is_file_command,
    spawn_save_panel,
};
use super::world_view::{LoadedWorld, enter_world};
use crate::building::data::BuildingType;
use crate::economy::economic_state::EconomicState;
use crate::economy::map_session::{
    ActorError, BuildRequestError, CancelRequestError, EconomySession, SessionPaths, scenario_by_id,
};
use crate::economy::map_state::{CancelError, CancelReceipt, ConstructionOrderId, DayReport, OrderError, SimDate};
use crate::economy::resources::ResourceType;
use crate::localization::{Locale, TranslationCatalog, translate};
use crate::world::{CellStatus, MapStateId, ProvinceId, WorldModel, cell_status};
use bevy::prelude::*;
use std::collections::{HashMap, VecDeque};

/// 1コマンドで進められる日数の上限。1日の処理(3ステート・3国家の建設・月次・AI評価)は軽く(300日分の連続実行を
/// 記録付きでテストが1秒未満で行える)、30日で1フレームに収まる。入力欄が無いので上限を超える指定は画面からは起きない。
pub const MAX_DAYS_PER_COMMAND: u32 = 30;
/// 進行ボタンの日数(いずれも `MAX_DAYS_PER_COMMAND` 以下)。
pub const ADVANCE_STEPS: [u32; 3] = [1, 7, 30];
const LOG_LINES: usize = 6;

// ---------------------------------------------------------------- コマンド

/// 画面から経済セッションへの操作要求。適用は [`apply_command`] の1経路だけ。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    /// 操作主体の変更(`None` = 選択解除)。**操作主体だけを変え**、経済状態・日付には触れない。
    SetActor(Option<u32>),
    /// `n` 日進める(1コマンド)。
    AdvanceDays(u32),
    /// 手動の建設注文(S2)。**ボタンを押した時点の操作主体・対象ステート・施設**をコマンドが保持し、適用時に
    /// 選択セルや一覧の位置からは読み直さない。適用時の操作主体が `actor` と違えば実行しない(古い操作を
    /// 別の国家の操作として実行しない)。受付判定は共通API(`EconomySession::request_build`)が現在の状態に対して行う。
    Build { actor: u32, state: MapStateId, building: BuildingType },
    /// 注文識別子を指定した建設注文のキャンセル(S2)。表示の行番号や位置ではなく**実際の注文識別子**で対象を指す。
    Cancel { actor: u32, order: ConstructionOrderId },
    /// 建設パネルの表示/非表示(画面だけの操作。経済状態には触れない)。
    ToggleBuildPanel,
    /// 保存(S3)。名前・操作主体・置換の明示は**ボタンを押した時点の値**。名前の検証・保存先の制限・原子的置換は共有API。
    /// `replace` が偽で同名の保存があれば拒否する(黙って置換しない)。
    SaveAs { actor: Option<u32>, name: String, replace: bool },
    /// 保存の事前確認(共有 `inspect`。復元はしない)。対象は保存名。
    InspectSave { name: String },
    /// 事前確認した保存からの復元(共有 `load` が最初から全検証する)。
    LoadSave { actor: Option<u32>, name: String },
    /// 保存一覧の更新(共有 `list_saves`。読み取りのみ)。
    RefreshSaves,
    /// 保存・復元パネルの表示/非表示、事前確認の詳細の表示/非表示(画面だけの操作)。
    ToggleSavePanel,
    ToggleSaveDetail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceSummary {
    pub days: u32,
    /// 進行前に最後に処理を完了していた日。
    pub from: SimDate,
    /// 進行後に最後に処理を完了した日。
    pub through: SimDate,
    /// (国家, ステート, 施設, 完成後のレベル)
    pub completed: Vec<(u32, MapStateId, BuildingType, u32)>,
    /// 完成条件を満たしたが前提が不足して保留された注文の数。
    pub blocked: usize,
    /// AIが出した注文 (国家, ステート, 施設, 支払った費用)
    pub ai_orders: Vec<(u32, MapStateId, BuildingType, f64)>,
    pub tax_changes: usize,
    /// 月次経済を実行した日の数。
    pub monthly_runs: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdvanceRejection {
    Actor(ActorError),
    /// 0日、または `MAX_DAYS_PER_COMMAND` を超える。
    InvalidDays(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandOutcome {
    ActorChanged(Option<u32>),
    ActorRejected(ActorError),
    Advanced(AdvanceSummary),
    AdvanceRejected(AdvanceRejection),
    /// 注文を受理した(支払い済み。内容は受理された注文そのもの)。
    Built {
        id: ConstructionOrderId,
        state: MapStateId,
        building: BuildingType,
        target_level: u32,
        cost: f64,
        required_progress: f64,
    },
    BuildRejected(BuildRequestError),
    /// キャンセルを受理した(返金の内訳は共通APIの結果)。
    Cancelled(CancelReceipt),
    CancelRejected(CancelRequestError),
    /// コマンドを出したときの操作主体と、適用時の操作主体が違う(国家の切替などで古くなった操作)。実行していない。
    ActorMismatch { commanded: u32, current: Option<u32> },
    BuildPanelToggled,
    /// 保存・復元系のコマンドは、地図・保存先の文脈を持つ `apply_session_commands`(`save_view::apply_file_command`)だけが適用する。
    FileCommandNeedsContext,
}

impl CommandOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(
            self,
            CommandOutcome::ActorChanged(_)
                | CommandOutcome::Advanced(_)
                | CommandOutcome::Built { .. }
                | CommandOutcome::Cancelled(_)
                | CommandOutcome::BuildPanelToggled
        )
    }
}

/// コマンドの適用(画面・起動時の `--actor` の両方がこの1つの関数を通る)。
/// 拒否した場合は `session`・`actor` を一切変更しない。
pub fn apply_command(session: &mut EconomySession, actor: &mut Option<u32>, cmd: SessionCommand) -> CommandOutcome {
    match cmd {
        SessionCommand::SetActor(None) => {
            *actor = None;
            CommandOutcome::ActorChanged(None)
        }
        SessionCommand::SetActor(Some(label)) => match session.resolve_actor(Some(label)) {
            Ok(l) => {
                *actor = Some(l);
                CommandOutcome::ActorChanged(Some(l))
            }
            Err(e) => CommandOutcome::ActorRejected(e),
        },
        SessionCommand::AdvanceDays(days) => {
            // 操作主体の検証(適用の時点・現在の選択に対して)→ 日数の検証 → 進行。
            if let Err(e) = session.resolve_actor(*actor) {
                return CommandOutcome::AdvanceRejected(AdvanceRejection::Actor(e));
            }
            if days == 0 || days > MAX_DAYS_PER_COMMAND {
                return CommandOutcome::AdvanceRejected(AdvanceRejection::InvalidDays(days));
            }
            let from = session.sim.date;
            let mut summary = AdvanceSummary {
                days,
                from,
                through: from,
                completed: Vec::new(),
                blocked: 0,
                ai_orders: Vec::new(),
                tax_changes: 0,
                monthly_runs: 0,
            };
            for _ in 0..days {
                let (date, report) = session.advance_day();
                summary.through = date;
                fold_report(&mut summary, report);
            }
            CommandOutcome::Advanced(summary)
        }
        SessionCommand::Build { actor: commanded, state, building } => {
            if *actor != Some(commanded) {
                return CommandOutcome::ActorMismatch { commanded, current: *actor };
            }
            // 判定(費用・鉱床・上限・重複・国家対応・クリスタル専用・Mineの資源設定)は共通APIが現在の状態に対して行う。
            match session.request_build(Some(commanded), state, building) {
                Ok(id) => {
                    let placed = session
                        .country(commanded)
                        .and_then(|c| c.construction_queue.iter().find(|o| o.id == id))
                        .expect("an accepted order is in the issuer's queue");
                    CommandOutcome::Built {
                        id,
                        state,
                        building,
                        target_level: placed.target_level,
                        cost: placed.paid_cost,
                        required_progress: placed.required_progress,
                    }
                }
                Err(e) => CommandOutcome::BuildRejected(e),
            }
        }
        SessionCommand::Cancel { actor: commanded, order } => {
            if *actor != Some(commanded) {
                return CommandOutcome::ActorMismatch { commanded, current: *actor };
            }
            // 返金・キューからの削除・二重取消しの拒否は共通APIが行う(UIでは何もしない)。
            match session.cancel_order(Some(commanded), order) {
                Ok(receipt) => CommandOutcome::Cancelled(receipt),
                Err(e) => CommandOutcome::CancelRejected(e),
            }
        }
        SessionCommand::ToggleBuildPanel => CommandOutcome::BuildPanelToggled,
        SessionCommand::SaveAs { .. }
        | SessionCommand::InspectSave { .. }
        | SessionCommand::LoadSave { .. }
        | SessionCommand::RefreshSaves
        | SessionCommand::ToggleSavePanel
        | SessionCommand::ToggleSaveDetail => CommandOutcome::FileCommandNeedsContext,
    }
}

fn fold_report(s: &mut AdvanceSummary, r: DayReport) {
    s.completed.extend(r.completed);
    s.blocked += r.blocked.len();
    s.ai_orders.extend(r.ordered);
    s.tax_changes += r.tax_changes.len();
    s.monthly_runs += r.monthly_ran as u32;
}

// ---------------------------------------------------------------- リソース

/// 共有セッション(経済状態の正本)。起動に失敗した場合は存在しない。
#[derive(Resource)]
pub struct SessionRes {
    pub session: EconomySession,
}

/// 画面の状態(経済値は持たない)。
#[derive(Resource, Debug)]
pub struct SessionUi {
    /// 明示的に選んだ操作主体。**既定は未選択**(列挙順の先頭などを選ばない)。
    pub actor: Option<u32>,
    /// 建設パネルを表示するか(画面だけの状態。既定は表示)。
    pub show_build: bool,
    /// 直近の操作結果(新しい順ではなく発生順。成功か否かと文言)。
    pub log: VecDeque<(bool, String)>,
}

impl Default for SessionUi {
    fn default() -> Self {
        SessionUi { actor: None, show_build: true, log: VecDeque::new() }
    }
}

impl SessionUi {
    pub fn push(&mut self, ok: bool, text: String) {
        self.log.push_back((ok, text));
        while self.log.len() > LOG_LINES {
            self.log.pop_front();
        }
    }
}

/// 起動時の設定(コマンドライン)。
#[derive(Resource, Debug, Clone)]
pub struct SessionConfig {
    pub scenario_id: String,
    pub actor: Option<u32>,
    pub paths: SessionPaths,
}

/// 起動に失敗した理由。これがある間は `SessionRes` が無く、状態を変える操作はできない。
#[derive(Resource, Debug, Clone)]
pub struct SessionLoadError(pub String);

/// 表示用の名前(資源名)。表示専用。
#[derive(Resource, Debug, Default)]
pub struct SessionLabels {
    pub resources: HashMap<ResourceType, String>,
}

impl SessionLabels {
    pub fn build() -> Self {
        let catalog = TranslationCatalog::load().ok();
        let resources = ResourceType::ALL
            .iter()
            .map(|r| {
                let name = match &catalog {
                    Some(c) => translate(c, Locale::JaJp, r.display_name(), &[]),
                    None => format!("{r:?}"),
                };
                (*r, name)
            })
            .collect();
        SessionLabels { resources }
    }
    fn resource(&self, r: ResourceType) -> String {
        self.resources.get(&r).cloned().unwrap_or_else(|| format!("{r:?}"))
    }
}

// ---------------------------------------------------------------- 選択セルの対象

/// 選択セルから導出した対象。**毎フレーム作り直す**ので、前のステートの参照は残らない。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SelectionTarget {
    #[default]
    None,
    /// 地図に存在しない座標(範囲外のインデックスなど)。
    NoCell,
    Water,
    OutOfScope,
    Unassigned { province: ProvinceId },
    State {
        state: MapStateId,
        province: ProvinceId,
        /// このステートが検証シナリオの経済に含まれるか。
        in_economy: bool,
        /// 経済計算上の対応国家(`ScenarioCountry.states` に含むもの。経済に含まれなければ空)。
        countries: Vec<u32>,
    },
}

#[derive(Resource, Debug, Default)]
pub struct SelectedView(pub SelectionTarget);

pub fn classify_selection(
    world: &WorldModel,
    map: &HexMap,
    selection: Option<u32>,
    session: &EconomySession,
) -> SelectionTarget {
    let Some(i) = selection else { return SelectionTarget::None };
    if (i as usize) >= map.cells().len() {
        return SelectionTarget::NoCell;
    }
    let c = map.cell_by_index(i);
    match cell_status(world, map, c.q, c.r) {
        CellStatus::NoCell => SelectionTarget::NoCell,
        CellStatus::Water => SelectionTarget::Water,
        CellStatus::OutOfScope => SelectionTarget::OutOfScope,
        CellStatus::Unassigned { province } => SelectionTarget::Unassigned { province },
        CellStatus::Assigned { province, state } => {
            let in_economy = session.state_economy(state).is_some();
            let countries = session.sim.countries.iter().filter(|c| c.states.contains(&state)).map(|c| c.label).collect();
            SelectionTarget::State { state, province, in_economy, countries }
        }
    }
}

// ---------------------------------------------------------------- 表示用テキスト(純粋関数)

pub(super) fn fmt_date(d: SimDate) -> String {
    format!("{:04}/{:02}/{:02}", d.year, d.month, d.day)
}

pub(super) fn country_label(session: &EconomySession, label: u32) -> String {
    match session.country(label) {
        Some(c) if c.ai.is_some() => format!("国家{label}(AI)"),
        Some(_) => format!("国家{label}(手動)"),
        None => format!("国家{label}"),
    }
}

pub(super) fn state_label(names: &HashMap<MapStateId, String>, id: MapStateId) -> String {
    match names.get(&id) {
        Some(n) => format!("{n}(ステート#{})", id.0),
        None => format!("ステート#{}", id.0),
    }
}

/// 最初の月次処理(開始月と別の月になった日)が済んでいるか。月次の値はそれまで未計算。
pub fn monthly_computed(start: SimDate, date: SimDate) -> bool {
    (date.year, date.month) != (start.year, start.month)
}

/// 日付の表示。`sim.date` は「最後に処理を完了した日」。開始直後は「未処理」。
pub fn date_text(start: SimDate, date: SimDate) -> String {
    let next = fmt_date(date.next().0);
    if date == start {
        format!("処理済み日付: 未処理(開始日 {})  次に処理する日: {next}", fmt_date(start))
    } else {
        format!("処理済み日付: {}  次に処理する日: {next}", fmt_date(date))
    }
}

fn actor_text(session: &EconomySession, actor: Option<u32>) -> String {
    match actor {
        Some(l) => format!("操作国家: {}", country_label(session, l)),
        None => "操作国家: 未選択".to_string(),
    }
}

/// 上部帯の見出し(日付・操作国家・シナリオ)。
pub fn header_text(session: &EconomySession, actor: Option<u32>) -> String {
    let sc = session.scenario();
    let hint = if actor.is_none() { "  ※日付を進めるには操作国家を選択してください" } else { "" };
    format!(
        "【検証用】新マップ 経済セッション  シナリオ: {}\n{}  /  {}{hint}\n(正式な世界設定・ゲーム開始ではありません)",
        sc.label,
        date_text(sc.start, session.sim.date),
        actor_text(session, actor),
    )
}

fn economic_state_label(s: EconomicState) -> &'static str {
    match s {
        EconomicState::Depression => "大恐慌",
        EconomicState::Recession => "不況",
        EconomicState::Stable => "安定",
        EconomicState::Boom => "好況",
        EconomicState::Prosperity => "繁栄",
    }
}

/// 国家の経済(国庫等)。月次の値は最初の月次処理まで「未計算」。
fn country_lines(session: &EconomySession, labels: &SessionLabels, label_no: u32) -> String {
    let Some(c) = session.country(label_no) else { return String::new() };
    let monthly = monthly_computed(session.scenario().start, session.sim.date);
    let mut t = format!(
        "{}\n国庫: {:.1}  税率: {:.1}%\n人的資源: 利用可能 {} / 動員済み {}",
        country_label(session, label_no),
        c.treasury,
        c.tax_rate * 100.0,
        c.available_manpower,
        c.mobilized_manpower
    );
    if monthly {
        t.push_str(&format!(
            "\n月次収支: 収入 {:.1} / 支出 {:.1} / 収支 {:+.1}\n経済状態: {}",
            c.monthly_income,
            c.monthly_expenses,
            c.monthly_balance,
            economic_state_label(c.economic_state)
        ));
    } else {
        t.push_str("\n月次収支・経済状態: 未計算(最初の月次処理前)");
    }
    let stock: Vec<String> = ResourceType::ALL
        .iter()
        .filter(|r| c.stockpile.get(**r) != 0.0)
        .map(|r| format!("{} {:.0}", labels.resource(*r), c.stockpile.get(*r)))
        .collect();
    t.push_str(&format!("\n備蓄: {}", if stock.is_empty() { "なし".to_string() } else { stock.join(" / ") }));
    t.push_str(&format!("\n建設キュー: {} 件", c.construction_queue.len()));
    t
}

/// ステートの経済(人口・雇用・施設)。雇用・物流・稼働率は最初の月次処理まで「未計算」。
fn state_economy_lines(session: &EconomySession, state: MapStateId) -> String {
    let Some(s) = session.state_economy(state) else { return String::new() };
    let monthly = monthly_computed(session.scenario().start, session.sim.date);
    let mut t = format!(
        "人口: {}\n労働力比率: {:.2}  生活水準: {:.1}  不満度: {:.1}",
        s.population, s.workforce_ratio, s.living_standard, s.unrest
    );
    if monthly {
        t.push_str(&format!(
            "\n雇用: {} / 失業: {}\n物流: 容量 {:.1} / 使用 {:.1} / 充足率 {:.2}",
            s.employed_workforce, s.unemployed_workforce, s.logistics_capacity, s.logistics_usage, s.logistics_ratio
        ));
    } else {
        t.push_str("\n雇用・失業・物流: 未計算(最初の月次処理前)");
    }
    t.push_str("\n施設:");
    let mut any = false;
    for b in BuildingType::ALL {
        let level = s.buildings.get(&b).copied().unwrap_or(0);
        if level == 0 {
            continue;
        }
        any = true;
        let name = session.buildings.get(b).map(|d| d.name.clone()).unwrap_or_else(|| format!("{b:?}"));
        let op = if monthly {
            match s.building_operation_ratios.get(&b) {
                Some(r) => format!(" 稼働率 {:.2}", r),
                None => String::new(),
            }
        } else {
            String::new()
        };
        t.push_str(&format!("\n  {name} Lv{level}{op}"));
    }
    if !any {
        t.push_str(" なし");
    }
    t
}

/// 右パネルの経済欄(操作国家・選択ステートの経済)。
pub fn economy_text(
    session: &EconomySession,
    labels: &SessionLabels,
    actor: Option<u32>,
    target: &SelectionTarget,
    names: &HashMap<MapStateId, String>,
) -> String {
    let mut t = "--- 経済セッション(検証用) ---\n".to_string();
    match actor {
        Some(a) => t.push_str(&country_lines(session, labels, a)),
        None => t.push_str("操作国家: 未選択(閲覧のみ。日付進行はできません)"),
    }
    t.push_str("\n\n--- 選択ステート ---\n");
    match target {
        SelectionTarget::None => t.push_str("選択なし(地図のセルを選択してください)"),
        SelectionTarget::NoCell => t.push_str("地図に存在しないセルです"),
        SelectionTarget::Water => t.push_str("水域(海・湖)— 経済の対象外"),
        SelectionTarget::OutOfScope => t.push_str("検証範囲外の陸セル — 経済データなし"),
        SelectionTarget::Unassigned { province } => {
            t.push_str(&format!("プロヴィンス{}: ステート未割当 — 経済データなし", province.0))
        }
        SelectionTarget::State { state, in_economy: false, .. } => t.push_str(&format!(
            "{}\nこの検証シナリオの経済には含まれていません",
            state_label(names, *state)
        )),
        SelectionTarget::State { state, countries, .. } => {
            t.push_str(&state_label(names, *state));
            let owners: Vec<String> = countries.iter().map(|c| country_label(session, *c)).collect();
            t.push_str(&format!("\n経済計算上の対応国家: {}", owners.join(", ")));
            t.push_str(match actor {
                None => "\n操作国家が未選択です(閲覧のみ)",
                Some(a) if countries.contains(&a) => "\n操作国家のステートです",
                Some(_) => "\n他国家のステートです(閲覧のみ。操作国家ではありません)",
            });
            t.push('\n');
            t.push_str(&state_economy_lines(session, *state));
        }
    }
    t.push_str("\n\n保存・復元は「保存・復元」パネルから(検証用セッションの mapeco 形式。旧ゲームのセーブとは別)。");
    t
}

pub(super) fn actor_error_text(e: &ActorError) -> String {
    match e {
        ActorError::NotSelected => "操作国家が未選択です。国家を選択してください".to_string(),
        ActorError::UnknownCountry(l) => format!("国家{l}はこのシナリオにありません"),
        ActorError::NotManual(l) => format!("国家{l}はAI国家のため、操作国家に選べません"),
        ActorError::StateNotMapped { actor, state } => {
            format!("ステート#{}は国家{actor}の経済に含まれません", state.0)
        }
    }
}

pub(super) fn building_name(session: &EconomySession, b: BuildingType) -> String {
    session.buildings.get(b).map(|d| d.name.clone()).unwrap_or_else(|| format!("{b:?}"))
}

/// 共通APIの拒否理由(`OrderError`)を、そのまま利用者向けの文言にする(別の理由へ置き換えない)。
pub(super) fn order_error_text(e: &OrderError, names: &HashMap<MapStateId, String>) -> String {
    match e {
        OrderError::UnknownCountry(l) => format!("国家{l}はこのシナリオにありません"),
        OrderError::UnknownState(s) => format!("{}はこの検証シナリオの経済にありません", state_label(names, *s)),
        OrderError::NotMappedToCountry(s) => {
            format!("{}は操作国家の経済計算に含まれていません(他国家のステート)", state_label(names, *s))
        }
        OrderError::UndefinedBuilding(_) => "この施設の定義がありません".to_string(),
        OrderError::UnsupportedFacility(_) => "ステート単位の経済施設ではありません".to_string(),
        OrderError::NoCrystalDeposit(_) => "発見済みのクリスタル鉱床がないため建設できません".to_string(),
        OrderError::CrystalOnlyState(_) => "クリスタル専用のステートには通常の鉱山を建設できません".to_string(),
        OrderError::MaxLevel { level, max } => format!("建設上限に達しています(Lv{level} / 上限 Lv{max})"),
        OrderError::AlreadyQueued => "同じ施設の注文が建設キューにあります".to_string(),
        OrderError::InsufficientFunds { cost, treasury } => {
            format!("国庫が足りません(費用 {cost:.0} / 国庫 {treasury:.1})")
        }
        OrderError::Precondition(b) => match b {
            crate::country::ai_core::OrderBlock::ResourcesNotConfigured => {
                "完成後の月次経済に必要な資源設定が未完了のため、注文できません".to_string()
            }
            crate::country::ai_core::OrderBlock::Other(s) => format!("注文の前提を満たしていません({s})"),
        },
    }
}

pub(super) fn build_error_text(e: &BuildRequestError, names: &HashMap<MapStateId, String>) -> String {
    match e {
        BuildRequestError::Actor(a) => actor_error_text(a),
        BuildRequestError::Order(o) => order_error_text(o, names),
    }
}

/// キャンセルの拒否理由(`CancelError`)。
pub(super) fn cancel_error_text(e: &CancelRequestError) -> String {
    match e {
        CancelRequestError::Actor(a) => actor_error_text(a),
        CancelRequestError::Cancel(c) => match c {
            CancelError::UnknownCountry(l) => format!("国家{l}はこのシナリオにありません"),
            CancelError::EmptyQueue => "取り消せる注文がありません".to_string(),
            CancelError::NotIssuersOrder(_) => "他国家の注文は取り消せません".to_string(),
            CancelError::AlreadyCompleted(_) => "完成済みの注文のため取り消せません(返金しません)".to_string(),
            CancelError::AlreadyCancelled(_) => "すでに取り消された注文です(二重に返金しません)".to_string(),
            CancelError::NoSuchOrder(_) => "この注文は存在しません".to_string(),
        },
    }
}

/// 操作結果の文言(成功か否か, 文)。
pub fn outcome_text(
    out: &CommandOutcome,
    session: &EconomySession,
    names: &HashMap<MapStateId, String>,
) -> (bool, String) {
    match out {
        CommandOutcome::ActorChanged(Some(l)) => (true, format!("操作国家を{}にしました", country_label(session, *l))),
        CommandOutcome::ActorChanged(None) => (true, "操作国家の選択を解除しました".to_string()),
        CommandOutcome::ActorRejected(e) => (false, format!("操作国家を変更できません: {}", actor_error_text(e))),
        CommandOutcome::AdvanceRejected(AdvanceRejection::Actor(e)) => {
            (false, format!("日付を進められません: {}", actor_error_text(e)))
        }
        CommandOutcome::AdvanceRejected(AdvanceRejection::InvalidDays(n)) => (
            false,
            format!("日付を進められません: 日数 {n} は範囲外です(1〜{MAX_DAYS_PER_COMMAND}日)"),
        ),
        CommandOutcome::Advanced(s) => {
            let mut t = format!(
                "{}日進めました({} → {} まで処理済み)。月次経済 {}回",
                s.days,
                fmt_date(s.from),
                fmt_date(s.through),
                s.monthly_runs
            );
            if s.monthly_runs > 0 {
                t.push_str("(収支・雇用・物流が更新されました)");
            }
            for (c, st, b, lv) in &s.completed {
                let n = session.buildings.get(*b).map(|d| d.name.clone()).unwrap_or_else(|| format!("{b:?}"));
                t.push_str(&format!("\n  完成: 国家{c} {} の {n} が Lv{lv} に", state_label(names, *st)));
            }
            for (c, st, b, cost) in &s.ai_orders {
                let n = session.buildings.get(*b).map(|d| d.name.clone()).unwrap_or_else(|| format!("{b:?}"));
                t.push_str(&format!("\n  AI発注: 国家{c} が {} に {n}(費用 {cost:.0})", state_label(names, *st)));
            }
            if s.blocked > 0 {
                t.push_str(&format!("\n  完成を保留された注文: {} 件", s.blocked));
            }
            (true, t)
        }
        CommandOutcome::Built { state, building, target_level, cost, required_progress, .. } => (
            true,
            format!(
                "発注しました: {} に {}(Lv{target_level}、費用 {cost:.0} を支払い、期間 {required_progress:.0}日)",
                state_label(names, *state),
                building_name(session, *building)
            ),
        ),
        CommandOutcome::BuildRejected(e) => (false, format!("発注できません: {}", build_error_text(e, names))),
        CommandOutcome::Cancelled(r) => (
            true,
            format!(
                "取り消しました: {} の {}(支払済み {:.0} のうち {:.0} を返金)",
                state_label(names, r.state),
                building_name(session, r.building_type),
                r.paid_cost,
                r.refund
            ),
        ),
        CommandOutcome::CancelRejected(e) => (false, format!("取り消せません: {}", cancel_error_text(e))),
        CommandOutcome::ActorMismatch { commanded, current } => (
            false,
            format!(
                "操作を実行しませんでした: 操作国家が変わっています(操作時: 国家{commanded} / 現在: {})",
                match current {
                    Some(c) => format!("国家{c}"),
                    None => "未選択".to_string(),
                }
            ),
        ),
        CommandOutcome::BuildPanelToggled => (true, "建設パネルの表示を切り替えました".to_string()),
        CommandOutcome::FileCommandNeedsContext => (false, "保存・復元の文脈がありません".to_string()),
    }
}

fn result_text(ui: &SessionUi, error: Option<&SessionLoadError>) -> String {
    if let Some(e) = error {
        return format!("セッションを開始できませんでした: {}", e.0);
    }
    if ui.log.is_empty() {
        return "直近の操作結果: なし".to_string();
    }
    let mut t = "直近の操作結果:".to_string();
    for (ok, line) in &ui.log {
        t.push_str(&format!("\n{} {line}", if *ok { "○" } else { "×" }));
    }
    t
}

// ---------------------------------------------------------------- システム(コア: ヘッドレスでも動く)

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionSet {
    Input,
    Apply,
    Target,
    Sync,
}

/// コマンドの受付・適用と選択対象の導出だけ(UI・描画なし)。ヘッドレスの統合テストはこれを使う。
pub struct EconomySessionCorePlugin;

impl Plugin for EconomySessionCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SessionCommand>()
            .init_resource::<SessionUi>()
            .init_resource::<SaveUi>()
            .init_resource::<SaveLocationRes>()
            .init_resource::<SelectedView>()
            .init_resource::<SessionLabels>()
            .configure_sets(
                Update,
                (SessionSet::Input, SessionSet::Apply, SessionSet::Target, SessionSet::Sync).chain(),
            )
            .add_systems(Update, apply_session_commands.in_set(SessionSet::Apply))
            .add_systems(Update, update_selection_target.in_set(SessionSet::Target));
    }
}

/// 到着順に1つずつ適用する(複数日進行は1コマンドとして途中に割り込ませない)。
/// 保存・復元のコマンドも同じ列で適用する。**復元が成功したら、その時点で受け付けた残りのコマンドは破棄する**
/// (復元前の古い発注・取消し・日付進行・保存が、復元後の状態へ適用されない)。
#[allow(clippy::too_many_arguments)]
pub fn apply_session_commands(
    mut reader: MessageReader<SessionCommand>,
    session: Option<ResMut<SessionRes>>,
    mut ui: ResMut<SessionUi>,
    world: Option<Res<LoadedWorld>>,
    map: Option<Res<LoadedHexMap>>,
    mut save: ResMut<SaveUi>,
    location: Res<SaveLocationRes>,
) {
    let commands: Vec<SessionCommand> = reader.read().cloned().collect();
    if commands.is_empty() {
        return;
    }
    let Some(mut session) = session else {
        for _ in commands {
            ui.push(false, "セッションがないため、操作できません".to_string());
        }
        return;
    };
    let empty = HashMap::new();
    let names = world.as_ref().map(|w| &w.names).unwrap_or(&empty);
    let mut iter = commands.into_iter();
    while let Some(cmd) = iter.next() {
        if is_file_command(&cmd) {
            let ctx = FileCtx {
                map: map.as_ref().map(|m| &m.0),
                world: world.as_ref().map(|w| &w.data.model),
                location: &location.0,
            };
            let r = apply_file_command(&mut session.session, &mut ui.actor, &mut save, &ctx, cmd);
            if r.restored {
                // 操作結果欄を整理(古い結果を残さない)し、残りの古いコマンドを破棄する。
                ui.log.clear();
            }
            if let Some((ok, text)) = r.text {
                ui.push(ok, text);
            }
            if r.restored {
                let dropped = iter.by_ref().count();
                if dropped > 0 {
                    ui.push(false, format!("復元前に受け付けた残りの操作 {dropped} 件は、復元後の状態へ適用せず破棄しました"));
                }
                break;
            }
            continue;
        }
        let out = apply_command(&mut session.session, &mut ui.actor, cmd);
        if out == CommandOutcome::BuildPanelToggled {
            ui.show_build = !ui.show_build; // 画面だけの状態(経済には触れない)
            continue;
        }
        if matches!(out, CommandOutcome::Advanced(_) | CommandOutcome::Built { .. } | CommandOutcome::Cancelled(_)) {
            save.state_changed(); // 経済状態が変わった: 事前確認は現在の結果ではなくなる
        }
        let (ok, text) = outcome_text(&out, &session.session, names);
        ui.push(ok, text);
    }
}

/// 選択セルから対象を毎フレーム導出する。地図・ワールド・セッションが無ければ「なし」(前の参照を残さない)。
pub fn update_selection_target(
    selection: Option<Res<HexSelection>>,
    map: Option<Res<LoadedHexMap>>,
    world: Option<Res<LoadedWorld>>,
    session: Option<Res<SessionRes>>,
    mut view: ResMut<SelectedView>,
) {
    let target = match (selection, map, world, session) {
        (Some(sel), Some(map), Some(world), Some(session)) => {
            classify_selection(&world.data.model, &map.0, sel.0, &session.session)
        }
        _ => SelectionTarget::None,
    };
    if view.0 != target {
        view.0 = target;
    }
}

// ---------------------------------------------------------------- 画面(UI・ライフサイクル)

#[derive(Component)]
struct HeaderText;
#[derive(Component)]
struct EconomyText;
#[derive(Component)]
struct ResultText;
/// 操作国家ボタン(国家はラベルで識別する。表示順は使わない)。
#[derive(Component)]
pub struct ActorButton(pub u32);
#[derive(Component)]
pub struct ClearActorButton;
/// 日付進行ボタン(日数を持つ)。
#[derive(Component)]
pub struct AdvanceButton(pub u32);

/// 経済セッションの画面(`HexPreviewPlugin` と一緒に追加する)。旧ゲームのプラグインは一切登録しない。
pub struct EconomySessionPlugin {
    pub config: SessionConfig,
}

impl Plugin for EconomySessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EconomySessionCorePlugin)
            .insert_resource(self.config.clone())
            .configure_sets(
                Update,
                (SessionSet::Input, SessionSet::Apply, SessionSet::Target, SessionSet::Sync)
                    .after(HexPreviewSet::Main)
                    .run_if(in_state(HexPreviewState::Open)),
            )
            .add_systems(
                OnEnter(HexPreviewState::Open),
                enter_session.after(enter_preview).after(enter_world),
            )
            .add_systems(OnExit(HexPreviewState::Open), exit_session)
            .add_systems(
                Update,
                block_close_key.before(HexPreviewSet::Main).run_if(in_state(HexPreviewState::Open)),
            )
            .add_systems(Update, session_buttons.in_set(SessionSet::Input))
            .add_systems(Update, (sync_session_text, sync_button_styles).in_set(SessionSet::Sync));
        add_build_panel_systems(app);
        add_save_panel_systems(app);
    }
}

/// `Tab`(確認画面を閉じる=セッション破棄)を、既存のキー処理が見る前に打ち消す。保存機能が無い間に
/// 状態を失う操作を作らないため。
pub fn block_close_key(mut keys: ResMut<ButtonInput<KeyCode>>) {
    keys.clear_just_pressed(KeyCode::Tab);
}

/// 起動時のセッション生成(共有初期化 `EconomySession::load_files` + `--actor` の検証)。失敗した場合は不完全な
/// セッションを返さず、理由だけを返す(別のシナリオ・国家へは置き換えない)。`--actor` は画面の国家選択と同じ
/// 検証([`apply_command`])を通る。成功時の `SessionUi` は、`--actor` があればその国家を選択済み、なければ**未選択**。
pub fn start_session(
    config: &SessionConfig,
    map_loaded: bool,
    world: Option<&WorldModel>,
) -> Result<(EconomySession, SessionUi), String> {
    if !map_loaded {
        return Err("地図を読み込めませんでした".to_string());
    }
    let Some(world) = world else {
        return Err("ワールド設定を読み込めませんでした".to_string());
    };
    let scenario = scenario_by_id(&config.scenario_id)
        .ok_or_else(|| format!("シナリオ \"{}\" は存在しません", config.scenario_id))?;
    let mut session = EconomySession::load_files(scenario, world, &config.paths).map_err(|e| e.to_string())?;
    let mut ui = SessionUi::default();
    if let Some(l) = config.actor {
        let out = apply_command(&mut session, &mut ui.actor, SessionCommand::SetActor(Some(l)));
        if let CommandOutcome::ActorRejected(e) = out {
            return Err(format!("--actor {l}: {}", actor_error_text(&e)));
        }
    }
    Ok((session, ui))
}

/// セッションを作る。失敗した場合は不完全なセッションを残さず、理由だけを保持する。
fn enter_session(
    mut commands: Commands,
    config: Res<SessionConfig>,
    map: Option<Res<LoadedHexMap>>,
    world: Option<Res<LoadedWorld>>,
    right: Query<Entity, With<RightPanel>>,
    status: Query<Entity, With<StatusText>>,
) {
    commands.insert_resource(SessionUi::default());
    commands.insert_resource(SaveUi::default());
    commands.insert_resource(SelectedView::default());
    commands.insert_resource(SessionLabels::build());
    let result = start_session(&config, map.is_some(), world.as_ref().map(|w| &w.data.model));
    match result {
        Ok((session, ui)) => {
            info!(
                "[EconomySession] scenario {} ready (manual actors: {:?}, actor: {:?})",
                session.scenario().id,
                session.manual_actors(),
                ui.actor
            );
            spawn_session_ui(&mut commands, Some(&session), right.single().ok(), status.single().ok());
            commands.insert_resource(ui);
            commands.insert_resource(SessionRes { session });
        }
        Err(e) => {
            error!("[EconomySession] failed to start: {e}");
            spawn_session_ui(&mut commands, None, right.single().ok(), status.single().ok());
            commands.insert_resource(SessionLoadError(e));
        }
    }
}

/// 画面を閉じる/終了するときの破棄(Entity は `HexPreviewEntity` の一括削除で消える)。ファイルは作らない。
fn exit_session(mut commands: Commands) {
    commands.remove_resource::<SessionRes>();
    commands.remove_resource::<SessionLoadError>();
    commands.insert_resource(SessionUi::default());
    commands.insert_resource(SaveUi::default());
    commands.insert_resource(SelectedView::default());
}

fn strip_node(top: bool) -> Node {
    let mut n = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(LEFT_PANEL_W),
        right: Val::Px(RIGHT_PANEL_W),
        flex_direction: FlexDirection::Column,
        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
        row_gap: Val::Px(4.0),
        ..default()
    };
    if top {
        n.top = Val::Px(0.0);
    } else {
        n.bottom = Val::Px(0.0);
    }
    n
}

fn row_node() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        column_gap: Val::Px(6.0),
        row_gap: Val::Px(4.0),
        ..default()
    }
}

fn spawn_session_ui(commands: &mut Commands, session: Option<&EconomySession>, right: Option<Entity>, status: Option<Entity>) {
    let bg = BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.92));
    // 上部帯(見出し・操作国家の選択・日付進行)。`Button` を付けて `Interaction` を発行させ、地図へ入力を渡さない。
    commands
        .spawn((strip_node(true), Button, bg, HexPreviewEntity))
        .with_children(|p| {
            p.spawn((HeaderText, label("", 13.0)));
            p.spawn(row_node()).with_children(|r| {
                r.spawn(label("操作国家:", 13.0));
                if let Some(s) = session {
                    // 国家の識別はラベル(ボタンの component)で行い、表示順は使わない。
                    let mut labels: Vec<u32> = s.sim.countries.iter().map(|c| c.label).collect();
                    labels.sort_unstable();
                    for l in labels {
                        r.spawn(button_bundle(ActorButton(l), &country_label(s, l)));
                    }
                }
                r.spawn(button_bundle(ClearActorButton, "選択解除"));
                r.spawn(button_bundle(ToggleBuildPanelButton, "建設パネル 表示/非表示"));
                r.spawn(button_bundle(ToggleSavePanelButton, "保存・復元 表示/非表示"));
                r.spawn(label("  日付を進める:", 13.0));
                for d in ADVANCE_STEPS {
                    r.spawn(button_bundle(AdvanceButton(d), &format!("+{d}日")));
                }
            });
        });
    // 建設パネル(S2。候補・注文・キュー・取消し)。セッションが無い場合は出さない。
    if session.is_some() {
        spawn_build_panel(commands);
        spawn_save_panel(commands);
    }
    // 下部帯(操作結果)。
    commands
        .spawn((strip_node(false), Button, bg, HexPreviewEntity))
        .with_children(|p| {
            p.spawn((ResultText, label("", 12.0)));
        });
    // 右パネルへ経済欄を追加。性能の状態欄(フレーム時間など)は経済セッションでは使わないので外す。
    if let Some(s) = status {
        commands.entity(s).despawn();
    }
    if let Some(panel) = right {
        let t = commands.spawn((EconomyText, label("", 12.0))).id();
        commands.entity(panel).add_child(t);
    }
}

/// ボタンを押した瞬間(`Interaction` が `Pressed` へ変わったフレーム)に1回だけコマンドを発行する。
/// 押し続けても `Changed` は発火しないので、同じ操作は重複しない。状態はここでは変えない。
#[allow(clippy::too_many_arguments)]
pub fn session_buttons(
    actors: Query<(&Interaction, &ActorButton), Changed<Interaction>>,
    clear: Query<&Interaction, (Changed<Interaction>, With<ClearActorButton>)>,
    advance: Query<(&Interaction, &AdvanceButton), Changed<Interaction>>,

    save: Option<Res<SaveUi>>,
    builds: Query<(&Interaction, &BuildButton), Changed<Interaction>>,
    cancels: Query<(&Interaction, &CancelButton), Changed<Interaction>>,
    toggle: Query<&Interaction, (Changed<Interaction>, With<ToggleBuildPanelButton>)>,
    mut out: MessageWriter<SessionCommand>,
) {
    for (i, b) in &actors {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::SetActor(Some(b.0)));
        }
    }
    for i in &clear {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::SetActor(None));
        }
    }
    for (i, b) in &advance {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::AdvanceDays(b.0));
        }
    }
    // 押した時点の操作国家・対象・注文識別子をそのままコマンドへ入れる(適用時に選択や位置から読み直さない)。
    let epoch = save.as_ref().map_or(0, |s| s.restore_epoch);
    for (i, b) in &builds {
        // 復元を挟んだ古い行(復元前の状態から作られたボタン)の操作は発行しない。
        if *i == Interaction::Pressed && b.epoch == epoch {
            out.write(SessionCommand::Build { actor: b.actor, state: b.state, building: b.building });
        }
    }
    for (i, b) in &cancels {
        if *i == Interaction::Pressed && b.epoch == epoch {
            out.write(SessionCommand::Cancel { actor: b.actor, order: b.order });
        }
    }
    for i in &toggle {
        if *i == Interaction::Pressed {
            out.write(SessionCommand::ToggleBuildPanel);
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_session_text(
    session: Option<Res<SessionRes>>,
    ui: Res<SessionUi>,
    view: Res<SelectedView>,
    labels: Res<SessionLabels>,
    world: Option<Res<LoadedWorld>>,
    error: Option<Res<SessionLoadError>>,
    mut header: Query<&mut Text, (With<HeaderText>, Without<EconomyText>, Without<ResultText>)>,
    mut economy: Query<&mut Text, (With<EconomyText>, Without<HeaderText>, Without<ResultText>)>,
    mut result: Query<&mut Text, (With<ResultText>, Without<HeaderText>, Without<EconomyText>)>,
) {
    let empty = HashMap::new();
    let names = world.as_ref().map(|w| &w.names).unwrap_or(&empty);
    if let Ok(mut t) = header.single_mut() {
        t.0 = match (&session, &error) {
            (Some(s), _) => header_text(&s.session, ui.actor),
            (None, Some(e)) => format!("【検証用】新マップ 経済セッション\n起動できませんでした: {}", e.0),
            (None, None) => "【検証用】新マップ 経済セッション".to_string(),
        };
    }
    if let (Ok(mut t), Some(s)) = (economy.single_mut(), &session) {
        t.0 = economy_text(&s.session, &labels, ui.actor, &view.0, names);
    }
    if let Ok(mut t) = result.single_mut() {
        t.0 = result_text(&ui, error.as_deref());
    }
}

fn sync_button_styles(
    ui: Res<SessionUi>,
    session: Option<Res<SessionRes>>,
    mut actors: Query<(&ActorButton, &mut BackgroundColor), Without<AdvanceButton>>,
    mut advance: Query<&mut BackgroundColor, (With<AdvanceButton>, Without<ActorButton>)>,
) {
    let selected = Color::srgb(0.2, 0.45, 0.3);
    let normal = Color::srgb(0.25, 0.27, 0.33);
    let dim = Color::srgb(0.16, 0.17, 0.2);
    for (b, mut bg) in &mut actors {
        let manual = session.as_ref().is_some_and(|s| s.session.resolve_actor(Some(b.0)).is_ok());
        bg.0 = if ui.actor == Some(b.0) {
            selected
        } else if manual {
            normal
        } else {
            dim // AI国家(選択できない)
        };
    }
    for mut bg in &mut advance {
        bg.0 = if ui.actor.is_some() { normal } else { dim };
    }
}
