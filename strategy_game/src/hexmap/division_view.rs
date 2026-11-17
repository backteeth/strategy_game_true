//! P32-039 (M6-f): 新マップの師団の表示と選択(`hex_map_preview --division-scenario <RON>`)。
//!
//! **シナリオの所有・支配・戦争関係は検証入力であり、正式な世界データではない。** 状態の正本は `world::MapMilitary`(P32-034〜038)で、この画面はそれを**読んで表示し、
//! 既存の `issue_move_order`・`advance_day` を呼ぶだけ**(移動・戦闘・占領・補給の再実装は無い)。旧ゲームのプラグインは使わない。保存・アセットは書かない。
//!
//! ## 構成
//! - [`DivisionSession`](純粋な Rust。Bevy 非依存): `MapMilitary` + `ProvinceGraph` + 検証用の法的所有 + 選択 + 日数(処理済みの日数)+ 操作結果。選択・命令・1日進行はすべてここのメソッド。
//! - [`DivisionCommand`](`Message`): ボタン・キーが発行し、[`apply_division_commands`] が**到着順に1つずつ**適用する。コマンドが無い限り日付も軍も進まない(描画更新・選択セル・表示モード・パン・ズームは進めない)。
//! - [`DivisionViewPlugin`]: 上記 + UI(上部帯・下部帯・右パネル欄)+ マーカー描画。`HexPreviewPlugin`(`--world-preview` 相当)と一緒に追加する。
//!
//! ## 表示
//! - 師団マーカー: プロヴィンスごとに 1 つ(同じプロヴィンスに複数いれば `xN` の件数)。位置は現在の `ProvinceId` の所属セルから決める(複数セルなら**重心に最も近いセル**。同距離は `(q, r)` が小さいほう。安定)。
//! - 支配の表示: 法的所有国(ステートの所有。検証入力)と異なる国が実効支配しているプロヴィンスに、小さな支配国の印を出す(法的所有と実効支配は別項目で文章にも出す)。
//! - 選択中の師団: 枠と、移動中なら目的地の印。
//!
//! ## 操作(画面にも表示)
//! - 左クリック: セル選択(既存)。上部帯の `師団N` ボタン、または `PageUp`/`PageDown`(MapDivisionId 順)で師団を選ぶ。`X` または「選択解除」で解除。
//! - 移動先は**地図で選んだセル**(左クリック)。`M` または「選択セルへ移動」で、選択中の師団へ `issue_move_order` を発行する(拒否の理由は下部帯に出し、既存の経路・進捗は変えない)。
//! - `T` または「選択セルのステートを選択師団の国へ割譲」で、選択中の師団の国が、選択セルのステートを併合する(P32-041 の領土移転。移転ステート内の進行中の戦闘は中止する。拒否の理由は下部帯)。
//! - `Space` / `N` または「1日進める」で `advance_day` を**ちょうど1回**。自動では進まない(検証モードは停止状態で始まる)。
//! - `Tab` は既存どおり画面を閉じる/開く。閉じる・終了で本セッションを破棄し、開き直すとシナリオを読み直して**初期配置**から始まる(古いマーカー・選択を持ち越さない)。
//!
//! パネル・帯・ボタンはすべて `Button`(`Interaction` を発行)なので、既存の `camera_input` の抑止(`ui_is_hovered`・`DragState.started_on_ui`)が効き、
//! UI 上のクリック・ドラッグ・スクロールは地図(セル選択・パン・ズーム)へ渡らない。

use super::data::HexMap;
use super::view::{
    HexPreviewEntity, HexPreviewSet, HexPreviewState, HexSelection, LEFT_PANEL_W, LoadedHexMap, RIGHT_PANEL_W, RightPanel, button_bundle, enter_preview, label,
};
use super::world_view::{LoadedWorld, enter_world};
use crate::common::CountryId;
use crate::world::{
    CountryStatus, DisbandReason, DivisionScenario, HexCoord, MapBattleStatus, MapDivision, MapDivisionId, MapDivisionStatus, MapMilitary, MoveEvent, OrderOutcome, ProvinceGraph, ProvinceId, StateControlKind,
    Territory, TransferContext, TransferOutcome, TransferRequest, WorldModel, load_division_scenario, summarize_control, transfer_state,
};
use bevy::prelude::*;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

const LOG_LINES: usize = 6;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------- 純粋な部分

/// プロヴィンスの表示位置(所属セルの中から安定して選ぶ)。重心に最も近いセル、同距離は `(q, r)` が小さいほう。セルが無ければ `None`。
pub fn display_cell(graph: &ProvinceGraph, province: ProvinceId) -> Option<HexCoord> {
    let cells = graph.cells_of(province)?;
    if cells.is_empty() {
        return None;
    }
    // 平面上の位置(尺度は無関係。最近傍の比較だけに使う)
    let pos = |c: &HexCoord| -> (f64, f64) { (3f64.sqrt() * (c.q as f64 + c.r as f64 / 2.0), 1.5 * c.r as f64) };
    let n = cells.len() as f64;
    let (sx, sy) = cells.iter().map(pos).fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (cx, cy) = (sx / n, sy / n);
    cells
        .iter()
        .min_by(|a, b| {
            let da = {
                let p = pos(a);
                (p.0 - cx).powi(2) + (p.1 - cy).powi(2)
            };
            let db = {
                let p = pos(b);
                (p.0 - cx).powi(2) + (p.1 - cy).powi(2)
            };
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal).then((a.q, a.r).cmp(&(b.q, b.r)))
        })
        .copied()
}

/// 1 プロヴィンスぶんの師団マーカー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceMarker {
    pub province: ProvinceId,
    pub cell: HexCoord,
    /// そのプロヴィンスにいる師団(`MapDivisionId` 昇順)。
    pub divisions: Vec<MapDivisionId>,
    /// 先頭の師団の所属国(色に使う)。
    pub first_owner: CountryId,
}

/// 法的所有国と異なる国が実効支配しているプロヴィンスの印。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMark {
    pub province: ProvinceId,
    pub cell: HexCoord,
    pub controller: CountryId,
    /// ステートの法的所有国(検証入力。ステート未所属・所有国なしは `None`)。
    pub legal_owner: Option<CountryId>,
}

/// 1 日進めた結果。
#[derive(Debug, Clone, PartialEq)]
pub struct DayResult {
    pub day: u32,
    pub events: Vec<MoveEvent>,
    /// 撃破されたため選択を解除した師団。
    pub deselected: Option<MapDivisionId>,
}

/// 検証用の師団セッション(Bevy 非依存)。
#[derive(Debug, Clone)]
pub struct DivisionSession {
    pub scenario_path: String,
    pub description: String,
    military: MapMilitary,
    graph: ProvinceGraph,
    /// 実行時の法的所有・首都の正本・国家の存続(M7-a。移転で変わる)。
    territory: Territory,
    /// `CountryId` の数値 → 国家ラベル(表示用)。
    labels: BTreeMap<usize, u32>,
    day: u32,
    selected: Option<MapDivisionId>,
    revision: u64,
    instance: u64,
    log: VecDeque<(bool, String)>,
}

impl DivisionSession {
    /// 検証用シナリオを読んで初期化する(既存のローダーを使う)。失敗は理由つき(ファイルが読めない・RON が不正・非対応の版・地図との不一致)。
    pub fn load(path: &str, graph: ProvinceGraph, model: &WorldModel) -> Result<DivisionSession, String> {
        let sc = load_division_scenario(path, &graph, model).map_err(|e| format!("{e}"))?;
        DivisionSession::from_scenario(path, sc, graph, model)
    }

    pub fn from_scenario(path: &str, sc: DivisionScenario, graph: ProvinceGraph, model: &WorldModel) -> Result<DivisionSession, String> {
        let labels = sc.countries.iter().map(|(l, c)| (c.0, *l)).collect();
        let territory = Territory::from_scenario(&sc, model, &graph).map_err(|e| e.to_string())?;
        let description = sc.description.clone();
        Ok(DivisionSession {
            scenario_path: path.to_string(),
            description,
            military: sc.into_military(),
            graph,
            territory,
            labels,
            day: 0,
            selected: None,
            revision: 0,
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            log: VecDeque::new(),
        })
    }

    pub fn military(&self) -> &MapMilitary {
        &self.military
    }
    pub fn graph(&self) -> &ProvinceGraph {
        &self.graph
    }
    /// 処理済みの日数(0 = 初期配置のまま)。
    pub fn day(&self) -> u32 {
        self.day
    }
    pub fn selected(&self) -> Option<MapDivisionId> {
        self.selected
    }
    /// 表示を作り直すべきかの判定に使う世代(軍・選択が変わるたびに増える)。
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// このセッションの識別(開き直すと別の値になる)。
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn log(&self) -> impl Iterator<Item = &(bool, String)> {
        self.log.iter()
    }
    /// 実行時の領土(法的所有・首都・国家の存続)。
    pub fn territory(&self) -> &Territory {
        &self.territory
    }

    fn push(&mut self, ok: bool, text: String) {
        self.log.push_back((ok, text));
        while self.log.len() > LOG_LINES {
            self.log.pop_front();
        }
    }

    pub fn country_text(&self, c: CountryId) -> String {
        match self.labels.get(&c.0) {
            Some(l) => format!("国家{l}"),
            None => format!("CountryId {}", c.0),
        }
    }

    /// 国家ラベル(シナリオの対応表に無ければ `None`)。
    pub fn label_of(&self, c: CountryId) -> Option<u32> {
        self.labels.get(&c.0).copied()
    }

    fn controller_text(&self, c: Option<CountryId>) -> String {
        c.map(|c| self.country_text(c)).unwrap_or_else(|| "なし".to_string())
    }

    /// 師団を選ぶ。存在しない ID は拒否して選択を変えない。
    pub fn select(&mut self, id: MapDivisionId) -> Result<(), String> {
        if self.military.division(id).is_none() {
            let t = format!("師団 {id} は存在しません(選択は変わりません)");
            self.push(false, t.clone());
            return Err(t);
        }
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.revision += 1;
        }
        self.push(true, format!("師団 {id} を選択しました"));
        Ok(())
    }

    pub fn clear_selection(&mut self) {
        if self.selected.take().is_some() {
            self.revision += 1;
            self.push(true, "選択を解除しました".to_string());
        }
    }

    /// 選択を MapDivisionId 順に前後へ動かす(未選択からは先頭/末尾。端では巡回する)。師団が無ければ何もしない。
    pub fn cycle(&mut self, forward: bool) {
        let ids: Vec<MapDivisionId> = self.military.divisions().map(|d| d.id).collect();
        if ids.is_empty() {
            return;
        }
        let next = match self.selected.and_then(|s| ids.iter().position(|i| *i == s)) {
            Some(i) => {
                if forward {
                    ids[(i + 1) % ids.len()]
                } else {
                    ids[(i + ids.len() - 1) % ids.len()]
                }
            }
            None => {
                if forward {
                    ids[0]
                } else {
                    ids[ids.len() - 1]
                }
            }
        };
        let _ = self.select(next);
    }

    /// 選択中の師団へ移動命令(既存の `issue_move_order`)。拒否は状態を変えず、理由を結果欄に出す。
    pub fn order_to(&mut self, destination: ProvinceId) -> Result<OrderOutcome, String> {
        let Some(id) = self.selected else {
            let t = "師団が選択されていないため、移動命令を出せません".to_string();
            self.push(false, t.clone());
            return Err(t);
        };
        match self.military.issue_move_order(&self.graph, id, destination) {
            Ok(o) => {
                let t = match &o {
                    OrderOutcome::Started { hops } => format!("命令を受理: {id} -> {destination}({hops} ホップ)"),
                    OrderOutcome::AlreadyThere => format!("命令なし: {id} は既に {destination} にいます"),
                    OrderOutcome::AlreadyMoving => format!("命令なし: {id} は既に {destination} へ移動中です"),
                };
                if matches!(o, OrderOutcome::Started { .. }) {
                    self.revision += 1;
                }
                self.push(true, t);
                Ok(o)
            }
            Err(r) => {
                let t = format!("命令を拒否: {id} -> {destination}: {r}(既存の経路・進捗は変わりません)");
                self.push(false, t.clone());
                Err(t)
            }
        }
    }

    /// 1 日進める(`advance_day` をちょうど 1 回)。撃破された師団が選択中なら、選択を解除する。
    pub fn advance_one_day(&mut self) -> DayResult {
        let events = self.military.advance_day(&self.graph);
        self.day += 1;
        self.revision += 1;
        let mut deselected = None;
        if let Some(s) = self.selected
            && self.military.division(s).is_none()
        {
            self.selected = None;
            deselected = Some(s);
        }
        self.push(true, format!("{} 日目を処理しました", self.day));
        let lines: Vec<String> = events.iter().filter_map(|e| self.event_text(e)).collect();
        for l in lines {
            self.push(true, format!("  {l}"));
        }
        if let Some(d) = deselected {
            self.push(false, format!("  選択中の {d} は撃破されたため、選択と詳細表示を解除しました"));
        }
        DayResult { day: self.day, events, deselected }
    }

    fn event_text(&self, e: &MoveEvent) -> Option<String> {
        Some(match e {
            MoveEvent::HopCompleted { division, province } => format!("{division} が {province} に到達"),
            MoveEvent::Arrived { division, province } => format!("{division} が目的地 {province} に到着"),
            MoveEvent::Stopped { division, at, blocked } => format!("{division} は {blocked} へ進めず {at} で停止"),
            MoveEvent::StoppedBefore { division, at, blocked, .. } => format!("{division} は {blocked} に入れず {at} で停止"),
            MoveEvent::PathTruncated { division } => format!("{division} の経路が途中で打ち切られました"),
            MoveEvent::BattleStarted { battle, province, attacker, defender, .. } => {
                format!("{battle} 開始 {province}: {} が {} を攻撃", self.country_text(*attacker), self.country_text(*defender))
            }
            MoveEvent::BattleJoined { battle, division, .. } => format!("{division} が {battle} に合流"),
            MoveEvent::Retreated { division, from, to, .. } => format!("{division} が {from} から {to} へ退却"),
            MoveEvent::Destroyed { division, at, .. } => format!("{division} が {at} で撃破"),
            MoveEvent::BattleFinished { battle, province, .. } => format!("{battle} 終了 {province}"),
            MoveEvent::ControlChanged { province, previous, new, .. } => {
                format!("支配が変わりました {province}: {} -> {}(法的所有は変わりません)", self.controller_text(*previous), self.country_text(*new))
            }
            MoveEvent::ClaimNotApplied { province, claimant, .. } => format!("占領を見送り {province}({}。自動では再適用されません)", self.country_text(*claimant)),
            MoveEvent::ControlClaim { .. } | MoveEvent::DailySupply(_) | MoveEvent::SupplyFailed { .. } => return None,
        })
    }

    /// 師団マーカー(プロヴィンス昇順)。表示セルが無いプロヴィンスの師団は含めない(通常は起きない)。
    pub fn markers(&self) -> Vec<ProvinceMarker> {
        let mut by_province: BTreeMap<ProvinceId, Vec<&MapDivision>> = BTreeMap::new();
        for d in self.military.divisions() {
            by_province.entry(d.province).or_default().push(d);
        }
        by_province
            .into_iter()
            .filter_map(|(province, mut ds)| {
                ds.sort_by_key(|d| d.id);
                let cell = display_cell(&self.graph, province)?;
                Some(ProvinceMarker { province, cell, divisions: ds.iter().map(|d| d.id).collect(), first_owner: ds[0].owner })
            })
            .collect()
    }

    /// 法的所有国と異なる国が実効支配しているプロヴィンス(プロヴィンス昇順)。
    pub fn control_marks(&self, model: &WorldModel) -> Vec<ControlMark> {
        self.graph
            .node_ids()
            .filter_map(|p| {
                let controller = self.military.control().controller(p)?;
                let legal_owner = model.state_of(p).and_then(|s| self.territory.owner(s));
                if legal_owner == Some(controller) {
                    return None;
                }
                Some(ControlMark { province: p, cell: display_cell(&self.graph, p)?, controller, legal_owner })
            })
            .collect()
    }

    /// 選択中の師団の目的地の表示セル(移動中だけ)。
    pub fn destination_cell(&self) -> Option<HexCoord> {
        let d = self.military.division(self.selected?)?;
        display_cell(&self.graph, d.destination?)
    }

    /// プロヴィンスにいる師団(`MapDivisionId` 昇順)。
    pub fn divisions_at(&self, province: ProvinceId) -> Vec<MapDivisionId> {
        self.military.divisions().filter(|d| d.province == province).map(|d| d.id).collect()
    }

    /// 師団の詳細(選択が無ければ未選択の案内)。法的所有国と実効支配国は別項目。
    pub fn detail_text(&self, model: &WorldModel) -> String {
        let Some(id) = self.selected else {
            return "師団: 未選択(上部の「師団N」または PageUp/PageDown で選びます)".to_string();
        };
        let Some(d) = self.military.division(id) else {
            return "師団: 未選択".to_string();
        };
        let state = model.state_of(d.province);
        let legal = state.and_then(|s| self.territory.owner(s));
        let mut t = format!(
            "師団: {}\n所属国: {}\n現在地: {}{}\n  法的所有国(ステート): {}\n  実効支配国(現在地): {}\n状態: {}",
            d.id,
            self.country_text(d.owner),
            d.province,
            state.map(|s| format!(" ({s})")).unwrap_or_else(|| " (ステート未所属)".to_string()),
            self.controller_text(legal),
            self.controller_text(self.military.control().controller(d.province)),
            status_text(d.status),
        );
        if d.status == MapDivisionStatus::Moving {
            t += &format!(
                "\n  目的地: {}\n  次のホップ先: {}\n  今のホップの進捗: {:.0}%(残りの経路 {} 件)",
                d.destination.map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
                d.target.or_else(|| d.path.first().copied()).map(|p| p.to_string()).unwrap_or_else(|| "?".into()),
                d.progress * 100.0,
                d.path.len()
            );
        }
        t += &format!(
            "\n人員 {}/{}  組織率 {:.1}/{:.1}  補給充足率 {:.2}",
            d.combat.manpower, d.combat.max_manpower, d.combat.organization, d.combat.max_organization, d.fulfillment
        );
        let here = self.divisions_at(d.province);
        if here.len() > 1 {
            t += &format!("\n同じプロヴィンスの師団: {}", here.iter().map(|i| i.0.to_string()).collect::<Vec<_>>().join(", "));
        }
        t
    }

    /// セルを選んだとき(そのセルのプロヴィンス)にいる師団の説明。師団がいなければ空文字。
    pub fn cell_divisions_text(&self, province: Option<ProvinceId>) -> String {
        let Some(p) = province else { return String::new() };
        let here = self.divisions_at(p);
        if here.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = here
            .iter()
            .filter_map(|id| self.military.division(*id))
            .map(|d| format!("{}({}, {})", d.id.0, self.country_text(d.owner), status_text(d.status)))
            .collect();
        format!("選択セルの師団 {} 件: {}", here.len(), parts.join(" / "))
    }

    /// ステート単位の支配(P32-038 の集計)。法的所有国と実効支配の内訳を別々に出す。
    pub fn control_text(&self, model: &WorldModel) -> String {
        let Ok(sum) = summarize_control(model, &self.territory.owners().collect(), self.military.control()) else {
            return "ステートの支配: 集計できません".to_string();
        };
        let mut t = String::from("ステート単位の所有と支配(検証入力。所有は領土移転で変わる):");
        for s in sum.states() {
            let owner = self.controller_text(s.legal_owner);
            let ctl = match s.kind() {
                StateControlKind::Full(c) => format!("全域 {}", self.country_text(c)),
                _ => {
                    let mut parts: Vec<String> = s.by_country.iter().map(|(c, n)| format!("{} {n}/{}", self.country_text(*c), s.total)).collect();
                    if s.uncontrolled > 0 {
                        parts.push(format!("支配国なし {}/{}", s.uncontrolled, s.total));
                    }
                    parts.join(", ")
                }
            };
            t += &format!("\n  {}: 法的所有 {owner} / 実効支配 {ctl}", s.state);
        }
        let caps: Vec<String> = self.territory.countries().filter_map(|c| self.territory.capital(c).map(|p| format!("{} {p}", self.country_text(c)))).collect();
        if !caps.is_empty() {
            t += &format!("\n首都(正本): {}", caps.join(" / "));
        }
        let gone: Vec<String> = self
            .territory
            .countries()
            .filter_map(|c| match self.territory.status(c) {
                Some(CountryStatus::Eliminated { annexed_by }) => Some(format!("{}(併合: {})", self.country_text(c), self.country_text(annexed_by))),
                _ => None,
            })
            .collect();
        if !gone.is_empty() {
            t += &format!("\n消滅した国家: {}", gone.join(" / "));
        }
        t
    }

    /// 領土移転(M7-a): 選択中の師団の国が、選択セルのステートを(現在の法的所有国から)併合する。移転ステート内の進行中の戦闘は中止の対象に明示する。
    /// 拒否は何も変えず、理由を結果欄に出す。適用したら、結果(支配・首都・師団・戦闘・消滅・補給)を結果欄に出し、選択中の師団が解散されたら選択を解除する。
    pub fn transfer_selected_cell_state(&mut self, model: &WorldModel, cell_province: Option<ProvinceId>) -> Result<TransferOutcome, String> {
        let fail = |this: &mut DivisionSession, t: String| -> Result<TransferOutcome, String> {
            this.push(false, t.clone());
            Err(t)
        };
        let Some(sel) = self.selected.and_then(|id| self.military.division(id)).map(|d| (d.id, d.owner)) else {
            return fail(self, "領土移転: 師団が選択されていません(選択した師団の国が、選択セルのステートを併合します)".to_string());
        };
        let Some(p) = cell_province else {
            return fail(self, "領土移転: 移転するステートのセルが選ばれていません".to_string());
        };
        let Some(state) = model.state_of(p) else {
            return fail(self, format!("領土移転: {p} はステートに所属していません"));
        };
        let Some(owner) = self.territory.owner(state) else {
            return fail(self, format!("領土移転: {state} には法的所有国がありません"));
        };
        let provinces: Vec<ProvinceId> = model.provinces_of(state).map(|v| v.to_vec()).unwrap_or_default();
        let battles: Vec<_> = self.military.battles().filter(|b| b.status == MapBattleStatus::Ongoing && provinces.contains(&b.province)).map(|b| b.id).collect();
        let req = TransferRequest::new(state, owner, sel.1).with_cancel_battles(battles);
        let ctx = TransferContext { model, graph: &self.graph };
        match transfer_state(ctx, &mut self.territory, &mut self.military, &req) {
            Err(e) => fail(self, format!("領土移転を拒否: {state}({} -> {}): {e}(所有・支配・師団・戦闘は変わりません)", self.country_text(owner), self.country_text(sel.1))),
            Ok(o) => {
                self.revision += 1;
                self.push(true, format!("領土移転: {state} の法的所有 {} -> {}(支配を変更 {} 件、第三国の占領を解消 {} 件)", self.country_text(o.previous_owner), self.country_text(o.new_owner), o.control_changes.len(), o.third_party_resolved.len()));
                if let Some(m) = &o.capital_move {
                    self.push(true, format!("  首都を移転: {} {} -> {}", self.country_text(m.country), m.from, m.to));
                }
                for b in &o.cancelled_battles {
                    self.push(true, format!("  戦闘を中止: {} {}(勝敗・占領なし。参加師団は待機)", b.battle, b.province));
                }
                if let Some(e) = &o.elimination {
                    self.push(true, format!("  国家が消滅: {}(併合: {}。IDは保持)", self.country_text(e.country), self.country_text(e.annexed_by)));
                }
                for r in &o.relocated {
                    self.push(true, format!("  師団{} が帰還: {} -> {}", r.division.0, r.from, r.to));
                }
                for d in &o.disbanded {
                    let why = match d.reason {
                        DisbandReason::CountryEliminated => "国家の消滅",
                        DisbandReason::NoReturnProvince => "帰還先なし",
                    };
                    self.push(true, format!("  師団{} を解散({why}。戦闘の撃破ではありません)", d.division.0));
                }
                if let Some(s) = self.selected
                    && self.military.division(s).is_none()
                {
                    self.selected = None;
                    self.push(false, format!("  選択中の {s} は解散されたため、選択と詳細表示を解除しました"));
                }
                Ok(o)
            }
        }
    }

    /// 上部の見出し。
    pub fn header_text(&self) -> String {
        let date = if self.day == 0 { "初期配置(未処理)".to_string() } else { format!("{} 日目まで処理済み", self.day) };
        format!("【検証用】新マップ 師団 (M6-f)  {date}  師団 {} 件\nシナリオ: {}", self.military.divisions().count(), self.scenario_path)
    }
}

pub fn status_text(s: MapDivisionStatus) -> &'static str {
    match s {
        MapDivisionStatus::Idle => "待機",
        MapDivisionStatus::Moving => "移動中",
        MapDivisionStatus::Fighting => "戦闘中",
    }
}

/// 国家ラベルごとの色(ラベルで決まる。表示順は使わない)。
pub fn country_color(label: Option<u32>) -> Color {
    const PALETTE: [(f32, f32, f32); 6] = [(0.85, 0.25, 0.25), (0.25, 0.45, 0.9), (0.2, 0.7, 0.35), (0.95, 0.65, 0.15), (0.65, 0.35, 0.85), (0.2, 0.75, 0.8)];
    match label {
        Some(l) if l >= 1 => {
            let (r, g, b) = PALETTE[(l as usize - 1) % PALETTE.len()];
            Color::srgb(r, g, b)
        }
        _ => Color::srgb(0.6, 0.6, 0.6),
    }
}

/// 選択セルのプロヴィンス(水域・範囲外・未選択は `None`)。
pub fn selected_province(model: &WorldModel, map: &HexMap, selection: Option<u32>) -> Option<ProvinceId> {
    let c = map.cells().get(selection? as usize)?;
    model.province_at(c.q, c.r).map(|p| p.id)
}

// ---------------------------------------------------------------- コマンド・システム(ヘッドレスでも動く)

#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum DivisionCommand {
    Select(MapDivisionId),
    ClearSelection,
    /// 選択中の師団の国が、選択セルのステートを併合する(領土移転。M7-a の検証用)。
    TransferSelectedCellState,
    /// 前(false)/次(true)の師団へ。
    Cycle(bool),
    /// 選択中の師団を、選択中のセルのプロヴィンスへ移動させる。
    OrderToSelectedCell,
    AdvanceDay,
}

#[derive(Resource)]
pub struct DivisionSessionRes(pub DivisionSession);

/// 起動に失敗した理由(画面表示用)。これがある間は師団のデータ・Entity は存在しない。
#[derive(Resource)]
pub struct DivisionLoadError(pub String);

#[derive(Resource, Debug, Clone)]
pub struct DivisionViewConfig {
    pub scenario: String,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum DivisionSet {
    Input,
    Apply,
    Sync,
}

/// コマンドの適用だけ(UI・描画なし)。ヘッドレスの統合テストはこれを使う。
pub struct DivisionViewCorePlugin;

impl Plugin for DivisionViewCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DivisionCommand>()
            .configure_sets(Update, (DivisionSet::Input, DivisionSet::Apply, DivisionSet::Sync).chain())
            .add_systems(Update, apply_division_commands.in_set(DivisionSet::Apply));
    }
}

/// 到着順に 1 つずつ適用する。`AdvanceDay` 1 件につき `advance_day` はちょうど 1 回。
pub fn apply_division_commands(
    mut reader: MessageReader<DivisionCommand>,
    session: Option<ResMut<DivisionSessionRes>>,
    world: Option<Res<LoadedWorld>>,
    map: Option<Res<LoadedHexMap>>,
    selection: Option<Res<HexSelection>>,
) {
    let commands: Vec<DivisionCommand> = reader.read().cloned().collect();
    let Some(mut session) = session else { return };
    for cmd in commands {
        let s = &mut session.0;
        match cmd {
            DivisionCommand::Select(id) => {
                let _ = s.select(id);
            }
            DivisionCommand::ClearSelection => s.clear_selection(),
            DivisionCommand::TransferSelectedCellState => match &world {
                Some(w) => {
                    let cell = match (&map, &selection) {
                        (Some(m), Some(sel)) => selected_province(&w.data.model, &m.0, sel.0),
                        _ => None,
                    };
                    let _ = s.transfer_selected_cell_state(&w.data.model, cell);
                }
                None => s.push(false, "領土移転: ワールド設定がありません".to_string()),
            },
            DivisionCommand::Cycle(fw) => s.cycle(fw),
            DivisionCommand::AdvanceDay => {
                s.advance_one_day();
            }
            DivisionCommand::OrderToSelectedCell => {
                let dest = match (&world, &map, &selection) {
                    (Some(w), Some(m), Some(sel)) => selected_province(&w.data.model, &m.0, sel.0),
                    _ => None,
                };
                match dest {
                    Some(p) => {
                        let _ = s.order_to(p);
                    }
                    None => s.push(false, "移動先のセルが選ばれていません(水域・範囲外・未選択のセルは移動先にできません)".to_string()),
                }
            }
        }
    }
}

// ---------------------------------------------------------------- 画面

#[derive(Component)]
pub struct DivisionButton(pub DivisionButtonKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivisionButtonKind {
    Advance,
    Order,
    Clear,
    Transfer,
    Prev,
    Next,
    Pick(MapDivisionId),
}

#[derive(Component)]
struct HeaderText;
#[derive(Component)]
struct ResultText;
#[derive(Component)]
struct DivisionText;
/// 師団・支配の描画物(世代が変わるたびに作り直す)。
#[derive(Component)]
pub struct DivisionMarkerEntity;
/// 上部帯・下部帯(`Button` を持つので、背後の地図へ入力が渡らない)。
#[derive(Component)]
pub struct DivisionStrip;

/// 画面(`HexPreviewPlugin`(ワールド設定つき)と一緒に追加する)。旧ゲームのプラグインは登録しない。
pub struct DivisionViewPlugin {
    pub config: DivisionViewConfig,
}

impl Plugin for DivisionViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DivisionViewCorePlugin)
            .insert_resource(self.config.clone())
            .configure_sets(Update, (DivisionSet::Input, DivisionSet::Apply, DivisionSet::Sync).after(HexPreviewSet::Main).run_if(in_state(HexPreviewState::Open)))
            .add_systems(OnEnter(HexPreviewState::Open), enter_division_view.after(enter_preview).after(enter_world))
            .add_systems(OnExit(HexPreviewState::Open), exit_division_view)
            .add_systems(Update, (division_buttons, division_keys).in_set(DivisionSet::Input))
            .add_systems(Update, (sync_division_markers, sync_division_text).chain().in_set(DivisionSet::Sync));
    }
}

fn enter_division_view(
    mut commands: Commands,
    config: Res<DivisionViewConfig>,
    map: Option<Res<LoadedHexMap>>,
    world: Option<Res<LoadedWorld>>,
    right: Query<Entity, With<RightPanel>>,
) {
    let result = match (&map, &world) {
        (Some(m), Some(w)) => ProvinceGraph::build(&w.data.model, &m.0)
            .map_err(|e| format!("プロヴィンスのグラフを作れませんでした: {e}"))
            .and_then(|g| DivisionSession::load(&config.scenario, g, &w.data.model)),
        (None, _) => Err("地図を読み込めませんでした".to_string()),
        (_, None) => Err("ワールド設定を読み込めませんでした(--world-preview が必要です)".to_string()),
    };
    match result {
        Ok(session) => {
            info!("[DivisionView] scenario {} ready ({} divisions)", session.scenario_path, session.military().divisions().count());
            spawn_division_ui(&mut commands, Some(&session), right.single().ok());
            commands.insert_resource(DivisionSessionRes(session));
        }
        Err(e) => {
            error!("[DivisionView] failed to start: {e}");
            spawn_division_ui(&mut commands, None, right.single().ok());
            commands.insert_resource(DivisionLoadError(format!("{}: {e}", config.scenario)));
        }
    }
}

/// 閉じる・終了するときの破棄(Entity は `HexPreviewEntity` の一括削除で消える)。開き直しは初期配置から。
pub fn exit_division_view(mut commands: Commands) {
    commands.remove_resource::<DivisionSessionRes>();
    commands.remove_resource::<DivisionLoadError>();
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
    Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, align_items: AlignItems::Center, column_gap: Val::Px(6.0), row_gap: Val::Px(4.0), ..default() }
}

/// 上部帯・下部帯・右パネルの欄を作る(`session` が `None` なら起動失敗の表示だけ)。帯は `Button` で、背後の地図へ入力を渡さない。
pub fn spawn_division_ui(commands: &mut Commands, session: Option<&DivisionSession>, right: Option<Entity>) {
    let bg = BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.92));
    commands.spawn((strip_node(true), Button, bg, HexPreviewEntity, DivisionStrip)).with_children(|p| {
        p.spawn((HeaderText, label("", 13.0)));
        if let Some(s) = session {
            p.spawn(row_node()).with_children(|r| {
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Advance), "1日進める (Space/N)"));
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Order), "選択セルへ移動 (M)"));
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Clear), "選択解除 (X)"));
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Transfer), "選択セルのステートを選択師団の国へ割譲 (T)"));
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Prev), "前の師団 (PgUp)"));
                r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Next), "次の師団 (PgDn)"));
            });
            p.spawn(row_node()).with_children(|r| {
                r.spawn(label("師団:", 13.0));
                for d in s.military().divisions() {
                    r.spawn(button_bundle(DivisionButton(DivisionButtonKind::Pick(d.id)), &format!("師団{} ({})", d.id.0, s.country_text(d.owner))));
                }
            });
        }
    });
    commands.spawn((strip_node(false), Button, bg, HexPreviewEntity, DivisionStrip)).with_children(|p| {
        p.spawn((ResultText, label("", 12.0)));
    });
    if let Some(panel) = right {
        let t = commands.spawn((DivisionText, label("", 12.0))).id();
        commands.entity(panel).add_child(t);
    }
}

/// ボタンを押した瞬間(`Interaction` が `Pressed` へ変わったフレーム)に 1 回だけコマンドを発行する。状態はここでは変えない。
pub fn division_buttons(buttons: Query<(&Interaction, &DivisionButton), Changed<Interaction>>, mut out: MessageWriter<DivisionCommand>) {
    for (i, b) in &buttons {
        if *i != Interaction::Pressed {
            continue;
        }
        out.write(match b.0 {
            DivisionButtonKind::Advance => DivisionCommand::AdvanceDay,
            DivisionButtonKind::Order => DivisionCommand::OrderToSelectedCell,
            DivisionButtonKind::Clear => DivisionCommand::ClearSelection,
            DivisionButtonKind::Transfer => DivisionCommand::TransferSelectedCellState,
            DivisionButtonKind::Prev => DivisionCommand::Cycle(false),
            DivisionButtonKind::Next => DivisionCommand::Cycle(true),
            DivisionButtonKind::Pick(id) => DivisionCommand::Select(id),
        });
    }
}

/// キー(押した瞬間に 1 回)。既存のキー(1〜4・G・R・B・F・Home・Tab・Esc)とは重ならない。
pub fn division_keys(keys: Res<ButtonInput<KeyCode>>, mut out: MessageWriter<DivisionCommand>) {
    if keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::KeyN) {
        out.write(DivisionCommand::AdvanceDay);
    }
    if keys.just_pressed(KeyCode::KeyM) {
        out.write(DivisionCommand::OrderToSelectedCell);
    }
    if keys.just_pressed(KeyCode::KeyX) {
        out.write(DivisionCommand::ClearSelection);
    }
    if keys.just_pressed(KeyCode::KeyT) {
        out.write(DivisionCommand::TransferSelectedCellState);
    }
    if keys.just_pressed(KeyCode::PageUp) {
        out.write(DivisionCommand::Cycle(false));
    }
    if keys.just_pressed(KeyCode::PageDown) {
        out.write(DivisionCommand::Cycle(true));
    }
}

/// 世代(セッションの識別 + 軍・選択の世代)が変わったときだけ、描画物を作り直す。日付・軍は進めない。
fn sync_division_markers(
    mut commands: Commands,
    session: Option<Res<DivisionSessionRes>>,
    world: Option<Res<LoadedWorld>>,
    map: Option<Res<LoadedHexMap>>,
    existing: Query<Entity, With<DivisionMarkerEntity>>,
    mut rendered: Local<Option<(u64, u64)>>,
) {
    let (Some(session), Some(world), Some(map)) = (session, world, map) else {
        return;
    };
    let s = &session.0;
    let key = (s.instance(), s.revision());
    if *rendered == Some(key) && !existing.is_empty() {
        return;
    }
    *rendered = Some(key);
    for e in &existing {
        commands.entity(e).despawn();
    }
    let size = map.0.grid.size as f32;
    let at = |c: HexCoord| -> Vec2 {
        let (x, y) = map.0.grid.center_world(c.q, c.r);
        Vec2::new(x as f32, y as f32)
    };
    let label_of = |c: CountryId| s.label_of(c);
    // 支配の印(法的所有と異なる実効支配)
    for m in s.control_marks(&world.data.model) {
        let p = at(m.cell);
        commands.spawn((
            Sprite { color: country_color(label_of(m.controller)), custom_size: Some(Vec2::splat(size * 0.5)), ..default() },
            Transform::from_xyz(p.x - size * 0.55, p.y - size * 0.55, 1.9),
            DivisionMarkerEntity,
            HexPreviewEntity,
        ));
    }
    // 選択中の師団の目的地
    if let Some(c) = s.destination_cell() {
        let p = at(c);
        commands.spawn((
            Sprite { color: Color::srgb(1.0, 1.0, 1.0), custom_size: Some(Vec2::splat(size * 0.7)), ..default() },
            Transform::from_xyz(p.x, p.y, 1.95).with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            DivisionMarkerEntity,
            HexPreviewEntity,
        ));
    }
    // 師団マーカー(プロヴィンスごとに 1 つ。複数なら件数)
    for m in s.markers() {
        let p = at(m.cell);
        let selected_here = s.selected().is_some_and(|id| m.divisions.contains(&id));
        if selected_here {
            commands.spawn((
                Sprite { color: Color::srgb(1.0, 0.95, 0.2), custom_size: Some(Vec2::splat(size * 1.25)), ..default() },
                Transform::from_xyz(p.x, p.y, 2.0),
                DivisionMarkerEntity,
                HexPreviewEntity,
            ));
        }
        commands.spawn((
            Sprite { color: country_color(label_of(m.first_owner)), custom_size: Some(Vec2::splat(size * 0.95)), ..default() },
            Transform::from_xyz(p.x, p.y, 2.05),
            DivisionMarkerEntity,
            HexPreviewEntity,
        ));
        if m.divisions.len() > 1 {
            commands.spawn((
                Text2d::new(format!("x{}", m.divisions.len())),
                // 文字は大きいフォントで作って縮小する(小さいフォントを拡大するとにじむ)。
                TextFont { font_size: FontSize::Px(64.0), ..default() },
                TextColor(Color::WHITE),
                Transform::from_xyz(p.x, p.y, 2.1).with_scale(Vec3::splat(size * 0.8 / 64.0)),
                DivisionMarkerEntity,
                HexPreviewEntity,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_division_text(
    session: Option<Res<DivisionSessionRes>>,
    error: Option<Res<DivisionLoadError>>,
    world: Option<Res<LoadedWorld>>,
    map: Option<Res<LoadedHexMap>>,
    selection: Option<Res<HexSelection>>,
    mut header: Query<&mut Text, (With<HeaderText>, Without<ResultText>, Without<DivisionText>)>,
    mut result: Query<&mut Text, (With<ResultText>, Without<HeaderText>, Without<DivisionText>)>,
    mut detail: Query<&mut Text, (With<DivisionText>, Without<HeaderText>, Without<ResultText>)>,
) {
    if let Ok(mut t) = header.single_mut() {
        t.0 = match (&session, &error) {
            (Some(s), _) => s.0.header_text(),
            (None, Some(e)) => format!("【検証用】新マップ 師団 (M6-f)\n起動できませんでした: {}", e.0),
            (None, None) => "【検証用】新マップ 師団 (M6-f)".to_string(),
        };
    }
    let (Some(s), Some(w)) = (&session, &world) else {
        if let Ok(mut t) = result.single_mut() {
            t.0 = error.as_ref().map(|e| format!("起動できませんでした: {}", e.0)).unwrap_or_default();
        }
        if let Ok(mut t) = detail.single_mut() {
            t.0.clear();
        }
        return;
    };
    if let Ok(mut t) = result.single_mut() {
        let lines: Vec<String> = s.0.log().map(|(ok, l)| format!("{}{l}", if *ok { "" } else { "× " })).collect();
        t.0 = if lines.is_empty() { "停止状態です。「1日進める」を押すまで日付も軍も進みません。".to_string() } else { lines.join("\n") };
    }
    if let Ok(mut t) = detail.single_mut() {
        let cell_prov = match (&map, &selection) {
            (Some(m), Some(sel)) => selected_province(&w.data.model, &m.0, sel.0),
            _ => None,
        };
        let mut text = format!("--- 師団(検証用) ---\n{}", s.0.detail_text(&w.data.model));
        let here = s.0.cell_divisions_text(cell_prov);
        if !here.is_empty() {
            text += &format!("\n{here}");
        }
        text += &format!("\n\n{}", s.0.control_text(&w.data.model));
        text += "\n\n操作: 師団ボタン/PgUp,PgDn=選択 X=解除 / 地図で移動先を左クリック→M=移動命令 / Space,N=1日進める / 地図でステートのセルを左クリック→T=選択師団の国へ割譲(領土移転)";
        t.0 = text;
    }
}
