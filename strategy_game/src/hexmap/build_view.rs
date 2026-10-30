//! P32-021 (S2): 経済施設の建設候補・注文・建設キュー・キャンセルのUI(独立バイナリ `map_economy_session` 用)。
//!
//! **判定は共有セッション(P32-019)の共通APIが行う。** 候補の受付可否は `EconomySession::check_build`
//! (状態を変更しない確認)、実際の注文は `request_build`、取消しは `cancel_order`(注文識別子指定)。費用・鉱床・上限・重複・
//! 国家対応・クリスタル専用・Mineの資源設定の判定をここへ複製しない。施設の一覧は既存の
//! `is_state_level_economic_facility` と建物定義(`BuildingRegistry`)から取り、UI独自の一覧を正本にしない。
//! 国庫・キュー・進捗・返金・完成は共有セッションのものだけで、UI用の別キューや別国庫は無い。
//!
//! ## モデルとUI
//! [`build_panel_model`] が現在の共有セッション・操作国家・選択対象から、表示する行(候補・キュー)を**純粋関数で**導出する
//! (テスト対象)。UI(`sync_build_panel`)はそのモデルから行を作り直すだけで、経済処理は再実行しない。作り直しは
//! セッション・操作国家・選択対象が変わったフレームだけ(`is_changed`)。
//!
//! ## 操作の対象
//! 発注ボタンは **押した時点の操作国家・対象ステート・施設** を([`BuildButton`])、取消しボタンは **操作国家と実際の注文識別子**
//! を([`CancelButton`])保持し、押した瞬間に1回だけ `SessionCommand` を発行する(`session_buttons`)。適用時に選択セルや
//! 行の位置から対象を読み直さないので、選択や国家が変わっても別のステート・別の注文へ操作が移らない。国家が切り替わって
//! 古くなった操作は `ActorMismatch` として実行しない。行は作り直しのたびに新しいEntity(`Interaction::None`)になるため、
//! 押し続け・再描画で同じ操作が再発行されることはない。UI独自の重複禁止は無い(重複は共通APIが `AlreadyQueued` で判断する)。
//!
//! ## 表示する進捗
//! 既存仕様では日次処理が各注文の進捗を **1日につき +1.0** 進め(`MapConstructionOrder::advance_one_day`)、
//! `required_progress`(建物定義)に達した日に完成する。したがって「進捗 p / 必要 r 日」「残り ⌈r−p⌉ 日」は
//! 既存仕様から正確に得られる値で、推測ではない。`p ≥ r` のまま注文がキューにある場合は、完成条件を満たしたが前提不足で
//! 保留されている(`DayReport.blocked`)状態として表示する。
//!
//! ## 入力
//! パネル全体が `Button`(`Interaction` を発行)なので、既存の `camera_input` の抑止が効き、パネル上のクリック・ドラッグ・
//! ホイールは地図へ渡らない。一覧は縦スクロール(`ui::scroll::{handle_panel_scroll, spawn_scrollable_body}` と
//! `ui::scrollbar::ScrollbarPlugin` を再利用)。

use super::economy_view::{
    SelectedView, SelectionTarget, SessionRes, SessionSet, SessionUi, build_error_text, building_name, country_label,
    state_label,
};
use super::save_view::SaveUi;
use super::view::{HexPreviewEntity, LEFT_PANEL_W, label};
use super::world_view::LoadedWorld;
use crate::building::construction::{ConstructionStatus, cancel_refund};
use crate::building::data::BuildingType;
use crate::economy::map_session::EconomySession;
use crate::economy::map_state::{ConstructionOrderId, MapConstructionOrder, OrderCheck, is_state_level_economic_facility};
use crate::world::MapStateId;
use bevy::prelude::*;
use std::collections::HashMap;

// ---------------------------------------------------------------- モデル(純粋関数)

/// 建設候補の1行。`order` が `Some` なら現在の状態で受理可能(ボタンを出す)、`None` なら `reason` が拒否理由。
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRow {
    pub building: BuildingType,
    pub text: String,
    /// 受理可能なときだけ `Some((発注する操作国家, 確認結果))`。
    pub order: Option<(u32, OrderCheck)>,
    pub reason: Option<String>,
}

/// 建設キューの1行。`cancel_by` が `Some` のときだけ取消しボタンを出す(自国家の注文)。他国家の注文は閲覧のみ。
#[derive(Debug, Clone, PartialEq)]
pub struct QueueRow {
    /// 実際の注文識別子(ボタンの操作対象。行番号ではない)。
    pub order: ConstructionOrderId,
    pub text: String,
    pub cancel_by: Option<u32>,
    /// 取り消したときの返金額の表示(共通の `cancel_refund` による。実際の返金は共通APIが行う)。
    pub refund: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueSection {
    pub title: String,
    pub rows: Vec<QueueRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildPanelModel {
    pub header: String,
    pub state: Option<MapStateId>,
    pub candidates: Vec<CandidateRow>,
    pub queues: Vec<QueueSection>,
}

fn status_text(o: &MapConstructionOrder) -> String {
    let remaining = (o.required_progress - o.progress).ceil().max(0.0);
    if o.progress >= o.required_progress {
        return "完成条件を満たしましたが、前提不足で完成を保留中".to_string();
    }
    let status = match o.status {
        ConstructionStatus::InQueue => "待機中(次の日から進行)",
        ConstructionStatus::InProgress => "建設中",
    };
    format!("{status}  進捗 {:.0}/{:.0}日(残り {remaining:.0}日)", o.progress, o.required_progress)
}

fn queue_rows(
    session: &EconomySession,
    names: &HashMap<MapStateId, String>,
    country: u32,
    cancel_by: Option<u32>,
) -> Vec<QueueRow> {
    let Some(c) = session.country(country) else { return Vec::new() };
    c.construction_queue
        .iter()
        .map(|o| QueueRow {
            order: o.id,
            text: format!(
                "{} Lv{}  {}\n{}  支払済み {:.0}",
                building_name(session, o.building_type),
                o.target_level,
                state_label(names, o.state),
                status_text(o),
                o.paid_cost
            ),
            cancel_by,
            refund: cancel_refund(o.paid_cost),
        })
        .collect()
}

/// 現在の共有セッション・操作国家・選択対象から、建設パネルの表示内容を導出する(状態を変更しない)。
/// 候補は選択ステートごとに毎回作り直すので、前のステートの候補・対象は残らない。
pub fn build_panel_model(
    session: &EconomySession,
    actor: Option<u32>,
    target: &SelectionTarget,
    names: &HashMap<MapStateId, String>,
) -> BuildPanelModel {
    let mut m = BuildPanelModel { header: String::new(), state: None, candidates: Vec::new(), queues: Vec::new() };
    let mut owner_for_view: Option<u32> = None;
    match target {
        SelectionTarget::None => m.header = "ステートを選択すると、建設候補を確認できます".to_string(),
        SelectionTarget::NoCell => m.header = "地図に存在しないセルです。建設できません".to_string(),
        SelectionTarget::Water => m.header = "水域(海・湖)には建設できません".to_string(),
        SelectionTarget::OutOfScope => m.header = "検証範囲外の陸セルには建設できません".to_string(),
        SelectionTarget::Unassigned { province } => {
            m.header = format!("プロヴィンス{}はステート未割当のため建設できません", province.0)
        }
        SelectionTarget::State { state, in_economy: false, .. } => {
            m.header = format!("{}はこの検証シナリオの経済に含まれないため、建設できません", state_label(names, *state))
        }
        SelectionTarget::State { state, countries, .. } => {
            m.state = Some(*state);
            owner_for_view = countries.first().copied();
            let note = match actor {
                None => "操作国家が未選択のため発注できません(候補の確認のみ)".to_string(),
                Some(a) if countries.contains(&a) => "発注する施設をクリックしてください".to_string(),
                Some(_) => "他国家のステートのため発注できません(閲覧のみ)".to_string(),
            };
            m.header = format!("{}\n{note}", state_label(names, *state));
            let eco = session.state_economy(*state);
            for b in BuildingType::ALL.into_iter().filter(|b| is_state_level_economic_facility(*b)) {
                let level = eco.and_then(|s| s.buildings.get(&b).copied()).unwrap_or(0);
                let def = session.buildings.get(b);
                let name = building_name(session, b);
                let spec = match def {
                    Some(d) => format!("費用 {:.0}  期間 {:.0}日", d.construction_cost, d.required_progress),
                    None => "定義なし".to_string(),
                };
                let head = format!("{name}  Lv{level}→Lv{}  {spec}", level + 1);
                // 受付可否は共通APIの確認(状態を変更しない)。実際の注文は押した時点で request_build が再検証する。
                match session.check_build(actor, *state, b) {
                    Ok(check) => m.candidates.push(CandidateRow {
                        building: b,
                        text: head,
                        order: actor.map(|a| (a, check)),
                        reason: None,
                    }),
                    Err(e) => {
                        let reason = build_error_text(&e, names);
                        m.candidates.push(CandidateRow {
                            building: b,
                            text: format!("{head}\n× {reason}"),
                            order: None,
                            reason: Some(reason),
                        });
                    }
                }
            }
        }
    }
    // キュー: 操作国家のキュー(取消し可)と、選択ステートの対応国家のキュー(自国家でなければ閲覧のみ)。
    if let Some(a) = actor.filter(|a| session.resolve_actor(Some(*a)).is_ok()) {
        m.queues.push(QueueSection {
            title: format!("{}の建設キュー", country_label(session, a)),
            rows: queue_rows(session, names, a, Some(a)),
        });
    }
    if let Some(o) = owner_for_view.filter(|o| Some(*o) != actor) {
        m.queues.push(QueueSection {
            title: format!("{}の建設キュー(閲覧のみ)", country_label(session, o)),
            rows: queue_rows(session, names, o, None),
        });
    }
    m
}

// ---------------------------------------------------------------- UI部品

/// 建設パネル全体(表示/非表示の切替対象)。
#[derive(Component)]
pub struct BuildPanelRoot;
#[derive(Component)]
pub struct BuildHeaderText;
/// 候補とキューの行を作り直す親(スクロール本体の子)。
#[derive(Component)]
pub struct BuildListRoot;
/// 発注ボタン。**押した時点の**操作国家・対象ステート・施設を持つ。
#[derive(Component)]
pub struct BuildButton {
    pub actor: u32,
    pub state: MapStateId,
    pub building: BuildingType,
    /// 行を作ったときの復元世代(`SaveUi::restore_epoch`)。復元を挟んだ古いボタンの操作は発行しない。
    pub epoch: u64,
}
/// 取消しボタン。**操作国家と実際の注文識別子**を持つ(行番号ではない)。
#[derive(Component)]
pub struct CancelButton {
    pub actor: u32,
    pub order: ConstructionOrderId,
    /// 行を作ったときの復元世代。復元で注文連番が戻っても、復元前の注文識別子の操作が別の注文へ作用しないための照合。
    pub epoch: u64,
}
/// 建設パネルの表示切替ボタン(上部帯)。
#[derive(Component)]
pub struct ToggleBuildPanelButton;

const PANEL_W: f32 = 360.0;

pub fn spawn_build_panel(commands: &mut Commands) {
    let bg = BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.94));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(LEFT_PANEL_W + 8.0),
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
            BuildPanelRoot,
            HexPreviewEntity,
        ))
        .with_children(|p| {
            p.spawn(label("建設(検証用)", 14.0));
            p.spawn((BuildHeaderText, label("", 12.0)));
            crate::ui::scroll::spawn_scrollable_body(p, Val::Px(4.0), |body| {
                body.spawn((
                    BuildListRoot,
                    Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), ..default() },
                ));
            });
        });
}

fn dim_label(text: String, size: f32) -> impl Bundle {
    (
        Text::new(text),
        TextFont { font_size: FontSize::Px(size), ..default() },
        TextColor(Color::srgb(0.62, 0.62, 0.66)),
    )
}

fn spawn_rows(list: &mut ChildSpawnerCommands, model: &BuildPanelModel, epoch: u64) {
    let normal = Color::srgb(0.22, 0.34, 0.27);
    let box_bg = Color::srgba(0.16, 0.17, 0.2, 0.9);
    if model.candidates.is_empty() {
        list.spawn(dim_label("(候補はありません)".to_string(), 12.0));
    } else {
        list.spawn(label("建設候補", 13.0));
    }
    for c in &model.candidates {
        match (&c.order, model.state) {
            (Some((actor, _)), Some(state)) => {
                list.spawn((
                    BuildButton { actor: *actor, state, building: c.building, epoch },
                    Button,
                    Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() },
                    BackgroundColor(normal),
                ))
                .with_children(|b| {
                    b.spawn(label(format!("発注: {}", c.text), 12.0));
                });
            }
            _ => {
                list.spawn((Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() }, BackgroundColor(box_bg)))
                    .with_children(|b| {
                        b.spawn(dim_label(c.text.clone(), 12.0));
                    });
            }
        }
    }
    for q in &model.queues {
        list.spawn(label(q.title.clone(), 13.0));
        if q.rows.is_empty() {
            list.spawn(dim_label("(注文はありません)".to_string(), 12.0));
        }
        for r in &q.rows {
            list.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                    ..default()
                },
                BackgroundColor(box_bg),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(r.text.clone()),
                    TextFont { font_size: FontSize::Px(12.0), ..default() },
                    TextColor(Color::srgb(0.92, 0.92, 0.92)),
                    Node { flex_grow: 1.0, flex_basis: Val::Px(0.0), ..default() },
                ));
                if let Some(actor) = r.cancel_by {
                    row.spawn((
                        CancelButton { actor, order: r.order, epoch },
                        Button,
                        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)), ..default() },
                        BackgroundColor(Color::srgb(0.4, 0.22, 0.22)),
                    ))
                    .with_children(|b| {
                        b.spawn(label(format!("取消\n返金 {:.0}", r.refund), 12.0));
                    });
                }
            });
        }
    }
}

/// 共有セッション・操作国家・選択対象が変わったフレームだけ、モデルから行を作り直す(経済処理は再実行しない)。
/// 作り直した行は新しいEntity(`Interaction::None`)で、押下状態を引き継がない。
#[allow(clippy::too_many_arguments)]
pub fn sync_build_panel(
    mut commands: Commands,
    session: Option<Res<SessionRes>>,
    ui: Res<SessionUi>,
    view: Res<SelectedView>,
    world: Option<Res<LoadedWorld>>,
    save: Option<Res<SaveUi>>,
    roots: Query<Entity, With<BuildListRoot>>,
    mut header: Query<&mut Text, With<BuildHeaderText>>,
) {
    let epoch = save.as_ref().map_or(0, |s| s.restore_epoch);
    let changed = session.as_ref().is_some_and(|s| s.is_changed()) || ui.is_changed() || view.is_changed();
    if !changed {
        return;
    }
    let empty = HashMap::new();
    let names = world.as_ref().map(|w| &w.names).unwrap_or(&empty);
    let model = match &session {
        Some(s) => build_panel_model(&s.session, ui.actor, &view.0, names),
        None => BuildPanelModel {
            header: "セッションを開始できなかったため、建設はできません".to_string(),
            state: None,
            candidates: Vec::new(),
            queues: Vec::new(),
        },
    };
    if let Ok(mut t) = header.single_mut() {
        t.0 = model.header.clone();
    }
    for root in &roots {
        commands.entity(root).despawn_related::<Children>();
        commands.entity(root).with_children(|list| spawn_rows(list, &model, epoch));
    }
}

/// 表示/非表示(画面だけの状態)。
pub fn sync_build_panel_visibility(ui: Res<SessionUi>, mut panel: Query<&mut Node, With<BuildPanelRoot>>) {
    if !ui.is_changed() {
        return;
    }
    for mut n in &mut panel {
        n.display = if ui.show_build { Display::Flex } else { Display::None };
    }
}

/// 建設パネルの更新をセッションの更新集合へ登録する。
pub fn add_build_panel_systems(app: &mut App) {
    app.add_plugins(crate::ui::scrollbar::ScrollbarPlugin)
        .add_systems(Update, crate::ui::scroll::handle_panel_scroll.in_set(SessionSet::Input))
        .add_systems(Update, (sync_build_panel, sync_build_panel_visibility).in_set(SessionSet::Sync));
}
