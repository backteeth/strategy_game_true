//! P32-001: ヘックスマップ確認画面(カメラ・表示モード・選択・UI)。
//!
//! 既存ゲームの`GameCamera`/`GameState`/州選択とは独立した専用画面。
//! `HexPreviewState::Open`の間だけ、専用カメラ・マップ・UIをspawnし、`Closed`へ戻る/再度開く際に
//! すべて(Entityとリソース)を破棄・再生成するため、開き直しても重複しない。
//! 読み込みに失敗した場合は、マップEntityを一切作らずエラーをUIに表示する。

use super::data::{self, Climate, HexMap, Terrain, Vegetation};
use super::geometry::{pick_cell_world, world_bounds};
use super::render::{
    self, HexViewMode, build_fill_mesh, build_grid_mesh, build_ring_mesh, build_river_mesh,
    plan_chunks,
};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::time::Instant;

pub const LEFT_PANEL_W: f32 = 250.0;
pub const RIGHT_PANEL_W: f32 = 280.0;
pub const MIN_ZOOM_SCALE: f32 = 0.04;
const MAX_ZOOM_OUT_FACTOR: f32 = 1.25;
const FIT_MARGIN: f32 = 1.04;
/// ヘックス境界線を表示するズーム上限(セル幅が約6px以上になるスケール)。
const GRID_VISIBLE_BELOW_SCALE: f32 = 0.7;
const CLICK_DRAG_THRESHOLD_PX: f32 = 4.0;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HexPreviewState {
    #[default]
    Closed,
    Open,
}

/// 確認画面のEntity全部に付ける印(破棄時の一括削除用)。
#[derive(Component)]
pub struct HexPreviewEntity;
#[derive(Component)]
pub struct HexCamera;
#[derive(Component)]
pub struct HexFillChunk {
    pub cells: Vec<u32>,
}
#[derive(Component)]
pub struct HexGridLines;
#[derive(Component)]
pub struct SelectionHalo;
#[derive(Component)]
pub struct SelectionRing;
#[derive(Component)]
struct LegendRoot;
#[derive(Component)]
struct InfoText;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
pub(super) struct ModeButton(pub(super) HexViewMode);
/// 左パネル(ワールド設定のUIを挿入する位置の目印)。
#[derive(Component)]
pub(super) struct LeftPanel;
#[derive(Component)]
struct FitButton;
#[derive(Component)]
struct GridButton;
#[derive(Component)]
struct RiverButton;
#[derive(Component)]
pub struct RiverLayer;

/// 読み込み済みマップ。確認画面を閉じると破棄する。
#[derive(Resource)]
pub struct LoadedHexMap(pub HexMap);

/// 読み込み失敗の内容(画面表示用)。
#[derive(Resource)]
pub struct HexLoadError(pub String);

#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct HexViewSettings {
    pub mode: HexViewMode,
    pub grid_enabled: bool,
    pub rivers_enabled: bool,
}

/// 選択中セルの配列インデックス(`LoadedHexMap`のインスタンス寿命の間のみ有効)。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexSelection(pub Option<u32>);

/// 読み込み・生成の計測値。
#[derive(Resource, Debug, Clone, Default)]
pub struct HexLoadStats {
    pub load_ms: f64,
    pub mesh_ms: f64,
    pub cells: usize,
    pub chunks: usize,
    pub rivers: usize,
    /// 正規化後の河川の辺(描画本数)。
    pub river_edges: usize,
    pub river_edges_open_border: usize,
    pub mesh_assets: usize,
    pub spawned_entities: usize,
}

/// ワールド座標の地図外接矩形と、全体表示要求フラグ。
#[derive(Resource)]
pub struct HexViewport {
    /// (min_x, min_y, max_x, max_y)
    pub bounds: (f32, f32, f32, f32),
    pub needs_fit: bool,
}

#[derive(Resource, Default)]
pub(super) struct DragState {
    pressed_at: Option<Vec2>,
    last_cursor: Option<Vec2>,
    dragging: bool,
    started_on_ui: bool,
}

#[derive(Resource, Default)]
struct FrameTimes {
    ema_ms: f32,
    max_ms: f32,
}

#[derive(Resource)]
struct SharedHandles {
    ring_mat: Handle<ColorMaterial>,
    halo_mat: Handle<ColorMaterial>,
    ring_mesh: Handle<Mesh>,
    halo_mesh: Handle<Mesh>,
    last_ring: Option<(Option<u32>, i32)>,
    river_mesh: Handle<Mesh>,
    casing_mesh: Handle<Mesh>,
    last_river_level: Option<i32>,
    river_rebuilds: usize,
}

type SelectionMarkers = Or<(With<SelectionHalo>, With<SelectionRing>)>;

pub struct HexPreviewPlugin {
    /// trueなら起動直後に確認画面を開く。
    pub open_at_start: bool,
    /// Some なら検証用ワールド設定(プロヴィンス表・ステート配置)を読み込んで重ねる(P32-005)。
    pub world_preview: Option<super::world_view::WorldPreviewPaths>,
}

impl Plugin for HexPreviewPlugin {
    fn build(&self, app: &mut App) {
        let open = self.open_at_start;
        if let Some(paths) = self.world_preview.clone() {
            app.add_plugins(super::world_view::WorldPreviewPlugin { paths });
        }
        app.init_state::<HexPreviewState>()
            .init_resource::<DragState>()
            .init_resource::<FrameTimes>()
            .add_systems(
                Startup,
                move |mut next: ResMut<NextState<HexPreviewState>>| {
                    if open {
                        next.set(HexPreviewState::Open);
                    }
                },
            )
            .add_systems(OnEnter(HexPreviewState::Open), enter_preview)
            .add_systems(OnExit(HexPreviewState::Open), exit_preview)
            .add_systems(
                Update,
                (
                    handle_buttons,
                    handle_keys,
                    camera_input,
                    apply_fit,
                    clamp_camera,
                    sync_mode,
                    sync_grid_visibility,
                    sync_rivers,
                    sync_selection_visual,
                    update_texts,
                )
                    .chain()
                    .run_if(in_state(HexPreviewState::Open)),
            )
            .add_systems(
                Update,
                closed_screen_keys.run_if(in_state(HexPreviewState::Closed)),
            );
    }
}

// ------------------------------------------------------------ 純粋関数(テスト対象)

/// カーソル(ウィンドウ論理座標, 左上原点Y下向き)→ワールド座標。
/// カメラの拡大はTransform.scale(既存コードベースと同じ)で表現する。
pub fn cursor_to_world(cursor: Vec2, window: Vec2, cam_pos: Vec2, scale: f32) -> Vec2 {
    let ndc = Vec2::new(
        cursor.x / window.x * 2.0 - 1.0,
        -(cursor.y / window.y * 2.0 - 1.0),
    );
    cam_pos + ndc * window * 0.5 * scale
}

/// 全体表示のカメラ位置とスケール。パネルを避けた空き領域に地図全体が収まるようにする。
/// 空き領域が小さすぎる(ウィンドウが極端に狭い)場合はウィンドウ全体を使う。
pub fn fit_camera(bounds: (f32, f32, f32, f32), window: Vec2) -> (Vec2, f32) {
    let free_w = window.x - LEFT_PANEL_W - RIGHT_PANEL_W;
    let (area_w, shift_px) = if free_w >= 100.0 {
        (free_w, (LEFT_PANEL_W - RIGHT_PANEL_W) * 0.5)
    } else {
        (window.x.max(1.0), 0.0)
    };
    let bw = (bounds.2 - bounds.0).max(1e-3);
    let bh = (bounds.3 - bounds.1).max(1e-3);
    let scale = (bw * FIT_MARGIN / area_w).max(bh * FIT_MARGIN / window.y.max(1.0));
    let centre = Vec2::new((bounds.0 + bounds.2) * 0.5, (bounds.1 + bounds.3) * 0.5);
    (Vec2::new(centre.x - shift_px * scale, centre.y), scale)
}

pub fn max_zoom_scale(bounds: (f32, f32, f32, f32), window: Vec2) -> f32 {
    fit_camera(bounds, window).1 * MAX_ZOOM_OUT_FACTOR
}

/// カメラ位置を地図の外接矩形内に、スケールを[最小, 最大]に収める。非有限値は全体表示値に戻す。
pub fn clamp_view(
    bounds: (f32, f32, f32, f32),
    window: Vec2,
    pos: Vec2,
    scale: f32,
) -> (Vec2, f32) {
    let (fit_pos, fit_scale) = fit_camera(bounds, window);
    if !pos.is_finite() || !scale.is_finite() {
        return (fit_pos, fit_scale);
    }
    let max_s = max_zoom_scale(bounds, window).max(MIN_ZOOM_SCALE);
    let s = scale.clamp(MIN_ZOOM_SCALE, max_s);
    (
        Vec2::new(
            pos.x.clamp(bounds.0, bounds.2),
            pos.y.clamp(bounds.1, bounds.3),
        ),
        s,
    )
}

/// カーソル位置のワールド点を固定したままスケールを変える。
pub fn zoom_about_cursor(cursor: Vec2, window: Vec2, cam_pos: Vec2, old: f32, new: f32) -> Vec2 {
    let anchor = cursor_to_world(cursor, window, cam_pos, old);
    let ndc = Vec2::new(
        cursor.x / window.x * 2.0 - 1.0,
        -(cursor.y / window.y * 2.0 - 1.0),
    );
    anchor - ndc * window * 0.5 * new
}

/// ズーム段階(scaleを2^0.5刻みに量子化)。
pub fn river_zoom_level(scale: f32) -> i32 {
    (scale.max(1e-6).log2() * 2.0).round() as i32
}

/// 段階ごとの河川の線幅(ワールド単位)。画面上でおよそ1.6pxを保ち、
/// 遠景ではセル一辺の4割(0.4・size)で頭打ちにして地図を覆わない。
pub fn river_width(size: f32, level: i32) -> f32 {
    let scale = 2f32.powf(level as f32 / 2.0);
    (1.6 * scale).min(0.4 * size)
}

/// 選択枠の半径(ワールド)。ズームしても約3px幅を保つ。
fn ring_thickness(size: f32, scale: f32) -> f32 {
    (3.0 * scale).max(size * 0.12)
}

// ------------------------------------------------------------ 開く/閉じる

#[allow(clippy::too_many_arguments)]
pub(super) fn enter_preview(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut drag: ResMut<DragState>,
) {
    *drag = DragState::default();
    commands.spawn((Camera2d, HexCamera, HexPreviewEntity, Transform::IDENTITY));
    spawn_ui(&mut commands);

    let t0 = Instant::now();
    let loaded = data::load_from_path(data::DEFAULT_MAP_PATH);
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let map = match loaded {
        Ok(m) => m,
        Err(e) => {
            error!("[HexPreview] failed to load hex map: {e}");
            commands.insert_resource(HexLoadError(e.to_string()));
            return;
        }
    };
    let Some(b) = world_bounds(&map) else {
        let msg = "map has no cells".to_string();
        error!("[HexPreview] {msg}");
        commands.insert_resource(HexLoadError(msg));
        return;
    };

    let settings = HexViewSettings {
        mode: HexViewMode::Terrain,
        grid_enabled: true,
        rivers_enabled: true,
    };
    let t1 = Instant::now();
    let chunks = plan_chunks(&map);
    let fill_mat = materials.add(ColorMaterial::from(Color::WHITE));
    let line_mat = materials.add(ColorMaterial::from(Color::srgba(0.0, 0.0, 0.0, 0.35)));
    let mut spawned = 0usize;
    let mut mesh_count = 0usize;
    for cells in &chunks {
        let fill = meshes.add(build_fill_mesh(&map, cells, settings.mode));
        commands.spawn((
            Mesh2d(fill),
            MeshMaterial2d(fill_mat.clone()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            HexFillChunk {
                cells: cells.clone(),
            },
            HexPreviewEntity,
        ));
        let lines = meshes.add(build_grid_mesh(&map, cells));
        commands.spawn((
            Mesh2d(lines),
            MeshMaterial2d(line_mat.clone()),
            Transform::from_xyz(0.0, 0.0, 1.0),
            HexGridLines,
            HexPreviewEntity,
        ));
        spawned += 2;
        mesh_count += 2;
    }

    // 選択枠(白い輪＋黒い縁取り)。メッシュはセル/ズームに応じて差し替える。
    let halo_mat = materials.add(ColorMaterial::from(Color::srgb(0.0, 0.0, 0.0)));
    let ring_mat = materials.add(ColorMaterial::from(Color::srgb(1.0, 0.95, 0.2)));
    let halo_mesh = meshes.add(build_ring_mesh((0.0, 0.0), 0.0, 0.0));
    let ring_mesh = meshes.add(build_ring_mesh((0.0, 0.0), 0.0, 0.0));
    commands.spawn((
        Mesh2d(halo_mesh.clone()),
        MeshMaterial2d(halo_mat.clone()),
        Transform::from_xyz(0.0, 0.0, 2.0),
        Visibility::Hidden,
        SelectionHalo,
        HexPreviewEntity,
    ));
    commands.spawn((
        Mesh2d(ring_mesh.clone()),
        MeshMaterial2d(ring_mat.clone()),
        Transform::from_xyz(0.0, 0.0, 3.0),
        Visibility::Hidden,
        SelectionRing,
        HexPreviewEntity,
    ));
    // 河川(縁取り+本体)。メッシュはズーム段階が変わった時だけ作り直す。
    let river_mat = materials.add(ColorMaterial::from(Color::srgb(0.25, 0.75, 1.0)));
    let casing_mat = materials.add(ColorMaterial::from(Color::srgb(0.02, 0.08, 0.2)));
    let river_mesh = meshes.add(build_river_mesh(&map, 0.0));
    let casing_mesh = meshes.add(build_river_mesh(&map, 0.0));
    commands.spawn((
        Mesh2d(casing_mesh.clone()),
        MeshMaterial2d(casing_mat),
        Transform::from_xyz(0.0, 0.0, 1.5),
        RiverLayer,
        HexPreviewEntity,
    ));
    commands.spawn((
        Mesh2d(river_mesh.clone()),
        MeshMaterial2d(river_mat),
        Transform::from_xyz(0.0, 0.0, 1.6),
        RiverLayer,
        HexPreviewEntity,
    ));
    spawned += 4;
    mesh_count += 4;
    let mesh_ms = t1.elapsed().as_secs_f64() * 1000.0;

    let stats = HexLoadStats {
        load_ms,
        mesh_ms,
        cells: map.cells().len(),
        chunks: chunks.len(),
        rivers: map.rivers().len(),
        river_edges: map.river_edges().len(),
        river_edges_open_border: map
            .river_edges()
            .iter()
            .filter(|e| !e.neighbor_exists)
            .count(),
        mesh_assets: mesh_count,
        spawned_entities: spawned,
    };
    info!(
        "[HexPreview] loaded {} cells / {} rivers in {:.1} ms; built {} chunks ({} meshes, {} map entities) in {:.1} ms",
        stats.cells,
        stats.rivers,
        stats.load_ms,
        stats.chunks,
        stats.mesh_assets,
        stats.spawned_entities,
        stats.mesh_ms
    );

    commands.insert_resource(SharedHandles {
        ring_mat,
        halo_mat,
        ring_mesh,
        halo_mesh,
        last_ring: None,
        river_mesh,
        casing_mesh,
        last_river_level: None,
        river_rebuilds: 0,
    });
    commands.insert_resource(stats);
    commands.insert_resource(settings);
    commands.insert_resource(HexSelection::default());
    commands.insert_resource(HexViewport {
        bounds: (b.0 as f32, b.1 as f32, b.2 as f32, b.3 as f32),
        needs_fit: true,
    });
    commands.insert_resource(LoadedHexMap(map));
}

fn exit_preview(mut commands: Commands, q: Query<Entity, With<HexPreviewEntity>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<LoadedHexMap>();
    commands.remove_resource::<HexLoadError>();
    commands.remove_resource::<HexViewSettings>();
    commands.remove_resource::<HexSelection>();
    commands.remove_resource::<HexViewport>();
    commands.remove_resource::<HexLoadStats>();
    commands.remove_resource::<SharedHandles>();
}

fn closed_screen_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<HexPreviewState>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next.set(HexPreviewState::Open);
    }
}

// ------------------------------------------------------------ UI

fn panel_node(left: bool) -> Node {
    let mut n = Node {
        position_type: PositionType::Absolute,
        top: Val::Px(0.0),
        bottom: Val::Px(0.0),
        width: Val::Px(if left { LEFT_PANEL_W } else { RIGHT_PANEL_W }),
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(Val::Px(10.0)),
        row_gap: Val::Px(6.0),
        ..default()
    };
    if left {
        n.left = Val::Px(0.0);
    } else {
        n.right = Val::Px(0.0);
    }
    n
}

pub(super) fn label(text: impl Into<String>, size: f32) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.92, 0.92)),
    )
}

pub(super) fn button_bundle<M: Component>(marker: M, text: &str) -> impl Bundle {
    (
        marker,
        Button,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.25, 0.27, 0.33)),
        children![label(text.to_string(), 14.0)],
    )
}

fn spawn_ui(commands: &mut Commands) {
    let bg = BackgroundColor(Color::srgba(0.08, 0.09, 0.12, 0.92));
    commands
        .spawn((panel_node(true), Button, bg, HexPreviewEntity, LeftPanel))
        .with_children(|p| {
            p.spawn(label("ヘックスマップ確認 (P32-001)", 15.0));
            for m in HexViewMode::ALL {
                p.spawn(button_bundle(ModeButton(m), m.label()));
            }
            p.spawn(button_bundle(FitButton, "全体表示 (Home)"));
            p.spawn(button_bundle(GridButton, "境界線 切替 (G)"));
            p.spawn(button_bundle(RiverButton, "河川 表示切替 (R)"));
            p.spawn((
                LegendRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    ..default()
                },
            ));
            p.spawn(label(
                "操作: ホイール=ズーム / 右・中・左ドラッグ=パン / 左クリック=選択\n1,2,3=モード  G=境界線  R=河川  (--world-preview: 4=ステート B=ステート境界 F=検証地域)  Tab=画面を閉じる/開く  Esc=終了",
                12.0,
            ));
        });
    commands
        .spawn((panel_node(false), Button, bg, HexPreviewEntity))
        .with_children(|p| {
            p.spawn((InfoText, label("", 14.0)));
            p.spawn((StatusText, label("", 12.0)));
        });
}

fn handle_buttons(
    modes: Query<(&Interaction, &ModeButton), Changed<Interaction>>,
    fits: Query<&Interaction, (Changed<Interaction>, With<FitButton>)>,
    grids: Query<&Interaction, (Changed<Interaction>, With<GridButton>)>,
    rivers: Query<&Interaction, (Changed<Interaction>, With<RiverButton>)>,
    settings: Option<ResMut<HexViewSettings>>,
    viewport: Option<ResMut<HexViewport>>,
) {
    if let Some(mut s) = settings {
        for (i, m) in &modes {
            if *i == Interaction::Pressed && s.mode != m.0 {
                s.mode = m.0;
            }
        }
        for i in &grids {
            if *i == Interaction::Pressed {
                s.grid_enabled = !s.grid_enabled;
            }
        }
        for i in &rivers {
            if *i == Interaction::Pressed {
                s.rivers_enabled = !s.rivers_enabled;
            }
        }
    }
    if let Some(mut v) = viewport {
        for i in &fits {
            if *i == Interaction::Pressed {
                v.needs_fit = true;
            }
        }
    }
}

fn handle_keys(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Option<ResMut<HexViewSettings>>,
    viewport: Option<ResMut<HexViewport>>,
    mut next: ResMut<NextState<HexPreviewState>>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        next.set(HexPreviewState::Closed);
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if let Some(mut s) = settings {
        for (key, m) in [
            (KeyCode::Digit1, HexViewMode::Terrain),
            (KeyCode::Digit2, HexViewMode::Climate),
            (KeyCode::Digit3, HexViewMode::Vegetation),
        ] {
            if keys.just_pressed(key) && s.mode != m {
                s.mode = m;
            }
        }
        if keys.just_pressed(KeyCode::KeyG) {
            s.grid_enabled = !s.grid_enabled;
        }
        if keys.just_pressed(KeyCode::KeyR) {
            s.rivers_enabled = !s.rivers_enabled;
        }
    }
    if let Some(mut v) = viewport
        && keys.just_pressed(KeyCode::Home)
    {
        v.needs_fit = true;
    }
}

// ------------------------------------------------------------ カメラ・選択

fn ui_is_hovered(ui: &Query<&Interaction>) -> bool {
    ui.iter()
        .any(|i| matches!(*i, Interaction::Hovered | Interaction::Pressed))
}

#[allow(clippy::too_many_arguments)]
fn camera_input(
    mut wheel: MessageReader<MouseWheel>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui: Query<&Interaction>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    mut drag: ResMut<DragState>,
    map: Option<Res<LoadedHexMap>>,
    viewport: Option<Res<HexViewport>>,
    mut selection: Option<ResMut<HexSelection>>,
) {
    let (Some(map), Some(viewport)) = (map, viewport) else {
        wheel.clear();
        return;
    };
    let (Ok(window), Ok(mut tf)) = (windows.single(), cam.single_mut()) else {
        wheel.clear();
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    if win.x < 1.0 || win.y < 1.0 {
        wheel.clear();
        return;
    }
    let over_ui = ui_is_hovered(&ui);
    let cursor = window.cursor_position();

    // ホイールズーム(UI上では地図へ渡さない)
    let mut scroll = 0.0_f32;
    for ev in wheel.read() {
        scroll += match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 50.0,
        };
    }
    if scroll != 0.0 && scroll.is_finite() && !over_ui {
        let old = tf.scale.x;
        let target = old * (-scroll * 0.15).exp();
        let (_, new) = clamp_view(viewport.bounds, win, tf.translation.truncate(), target);
        let pos = match cursor {
            Some(c) => zoom_about_cursor(c, win, tf.translation.truncate(), old, new),
            None => tf.translation.truncate(),
        };
        tf.translation.x = pos.x;
        tf.translation.y = pos.y;
        tf.scale = Vec3::splat(new);
    }

    // ドラッグ(左/右/中)でパン、閾値内の左クリックは選択
    let any_pressed =
        buttons.any_just_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]);
    if any_pressed {
        drag.pressed_at = cursor;
        drag.last_cursor = cursor;
        drag.dragging = false;
        drag.started_on_ui = over_ui;
    }
    let held = buttons.any_pressed([MouseButton::Left, MouseButton::Right, MouseButton::Middle]);
    if held
        && !drag.started_on_ui
        && let (Some(c), Some(last), Some(start)) = (cursor, drag.last_cursor, drag.pressed_at)
    {
        if !drag.dragging && c.distance(start) > CLICK_DRAG_THRESHOLD_PX {
            drag.dragging = true;
        }
        if drag.dragging {
            let d = c - last;
            tf.translation.x -= d.x * tf.scale.x;
            tf.translation.y += d.y * tf.scale.x;
        }
        drag.last_cursor = Some(c);
    }
    if buttons.just_released(MouseButton::Left)
        && !drag.dragging
        && !drag.started_on_ui
        && !over_ui
        && let (Some(c), Some(sel)) = (cursor, selection.as_mut())
    {
        let w = cursor_to_world(c, win, tf.translation.truncate(), tf.scale.x);
        sel.0 = pick_cell_world(&map.0, w.x as f64, w.y as f64);
    }
    if !held {
        drag.pressed_at = None;
        drag.last_cursor = None;
        drag.dragging = false;
        drag.started_on_ui = false;
    }
}

pub(super) fn apply_fit(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    viewport: Option<ResMut<HexViewport>>,
) {
    let Some(mut vp) = viewport else { return };
    if !vp.needs_fit {
        return;
    }
    let (Ok(window), Ok(mut tf)) = (windows.single(), cam.single_mut()) else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    if win.x < 1.0 || win.y < 1.0 {
        return;
    }
    let (pos, scale) = fit_camera(vp.bounds, win);
    tf.translation.x = pos.x;
    tf.translation.y = pos.y;
    tf.scale = Vec3::splat(scale);
    vp.needs_fit = false;
}

fn clamp_camera(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    viewport: Option<Res<HexViewport>>,
) {
    let Some(vp) = viewport else { return };
    let (Ok(window), Ok(mut tf)) = (windows.single(), cam.single_mut()) else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    if win.x < 1.0 || win.y < 1.0 {
        return;
    }
    let (pos, scale) = clamp_view(vp.bounds, win, tf.translation.truncate(), tf.scale.x);
    tf.translation.x = pos.x;
    tf.translation.y = pos.y;
    tf.scale = Vec3::splat(scale);
}

// ------------------------------------------------------------ 表示同期

#[allow(clippy::too_many_arguments)]
fn sync_mode(
    mut commands: Commands,
    settings: Option<Res<HexViewSettings>>,
    map: Option<Res<LoadedHexMap>>,
    chunks: Query<(&Mesh2d, &HexFillChunk)>,
    mut meshes: ResMut<Assets<Mesh>>,
    legend_root: Query<Entity, With<LegendRoot>>,
    mut last_mode: Local<Option<HexViewMode>>,
    loaded_flag: Option<Res<HexLoadStats>>,
    world: Option<Res<super::world_view::LoadedWorld>>,
) {
    let (Some(settings), Some(map), Some(_)) = (settings, map, loaded_flag) else {
        *last_mode = None;
        return;
    };
    if *last_mode == Some(settings.mode) {
        return;
    }
    *last_mode = Some(settings.mode);
    // ジオメトリはそのまま、頂点色だけ差し替える。
    for (m, chunk) in &chunks {
        if let Some(mut mesh) = meshes.get_mut(&m.0) {
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                render::vertex_colors_world(
                    &map.0,
                    &chunk.cells,
                    settings.mode,
                    world.as_ref().map(|w| &w.data.model),
                ),
            );
        }
    }
    let counts = map.0.attribute_counts();
    let Ok(root) = legend_root.single() else {
        return;
    };
    commands.entity(root).despawn_children();
    commands.entity(root).with_children(|p| {
        p.spawn(label(format!("凡例: {}", settings.mode.label()), 14.0));
        let entries = match (settings.mode, world.as_ref()) {
            (HexViewMode::State, Some(w)) => super::world_view::state_legend(w, &map.0),
            (HexViewMode::State, None) => vec![(
                "ワールド設定なし(--world-preview)".to_string(),
                [0.5, 0.5, 0.5],
            )],
            _ => render::legend(settings.mode),
        };
        for (name, color) in entries {
            let n = legend_count(settings.mode, &name, &counts);
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        width: Val::Px(14.0),
                        height: Val::Px(14.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(color[0], color[1], color[2])),
                ));
                row.spawn(label(format!("{name}{n}"), 12.0));
            });
        }
    });
}

/// 凡例行の件数表示。水域行(海/湖)は地形別件数、他は属性別件数。
fn legend_count(mode: HexViewMode, name: &str, c: &data::AttributeCounts) -> String {
    let find = |label: &str| -> Option<usize> {
        match mode {
            HexViewMode::Terrain => Terrain::ALL
                .iter()
                .position(|t| render::terrain_label(*t) == label)
                .map(|i| c.terrain[i]),
            HexViewMode::Climate => Climate::ALL
                .iter()
                .position(|t| render::climate_label(*t) == label)
                .map(|i| c.climate[i]),
            HexViewMode::Vegetation => Vegetation::ALL
                .iter()
                .position(|t| render::vegetation_label(*t) == label)
                .map(|i| c.vegetation[i]),
            HexViewMode::State => None,
        }
    };
    let n = if name == "未分類(陸上)" {
        match mode {
            HexViewMode::Climate => Some(c.land_climate_unclassified),
            HexViewMode::Vegetation => Some(c.land_vegetation_unclassified),
            HexViewMode::Terrain | HexViewMode::State => None,
        }
    } else if name.starts_with("海") {
        Some(c.terrain[0])
    } else if name.starts_with("湖") && mode != HexViewMode::Terrain {
        Some(c.terrain[1])
    } else {
        find(name)
    };
    n.map(|n| format!(" ({n})")).unwrap_or_default()
}

fn sync_grid_visibility(
    settings: Option<Res<HexViewSettings>>,
    cam: Query<&Transform, With<HexCamera>>,
    mut lines: Query<&mut Visibility, With<HexGridLines>>,
) {
    let (Some(settings), Ok(tf)) = (settings, cam.single()) else {
        return;
    };
    let want = if settings.grid_enabled && tf.scale.x < GRID_VISIBLE_BELOW_SCALE {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut lines {
        if *v != want {
            *v = want;
        }
    }
}

/// 河川の表示切替と、ズーム段階(scaleの2乗根刻み)が変わった時だけ線幅を作り直す。
/// パン中・同一段階のズーム中は何も再構築しない。
fn sync_rivers(
    settings: Option<Res<HexViewSettings>>,
    map: Option<Res<LoadedHexMap>>,
    handles: Option<ResMut<SharedHandles>>,
    cam: Query<&Transform, With<HexCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut layers: Query<&mut Visibility, With<RiverLayer>>,
) {
    let (Some(settings), Some(map), Some(mut h), Ok(tf)) = (settings, map, handles, cam.single())
    else {
        return;
    };
    let want = if settings.rivers_enabled {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut layers {
        if *v != want {
            *v = want;
        }
    }
    let level = river_zoom_level(tf.scale.x);
    if h.last_river_level == Some(level) {
        return;
    }
    h.last_river_level = Some(level);
    h.river_rebuilds += 1;
    let w = river_width(map.0.grid.size as f32, level);
    if let Some(mut m) = meshes.get_mut(&h.river_mesh) {
        *m = build_river_mesh(&map.0, w as f64);
    }
    if let Some(mut m) = meshes.get_mut(&h.casing_mesh) {
        *m = build_river_mesh(&map.0, (w * 1.9) as f64);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_selection_visual(
    map: Option<Res<LoadedHexMap>>,
    selection: Option<Res<HexSelection>>,
    handles: Option<ResMut<SharedHandles>>,
    cam: Query<&Transform, With<HexCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut vis: Query<&mut Visibility, SelectionMarkers>,
) {
    let (Some(map), Some(selection), Some(mut h), Ok(tf)) = (map, selection, handles, cam.single())
    else {
        return;
    };
    let _ = (&h.ring_mat, &h.halo_mat);
    // 太さを量子化して、ズーム中だけ小さなメッシュを更新する。
    let thick = ring_thickness(map.0.grid.size as f32, tf.scale.x);
    let key = (selection.0, (thick * 20.0).round() as i32);
    if h.last_ring == Some(key) {
        return;
    }
    h.last_ring = Some(key);
    match selection.0 {
        None => {
            for mut v in &mut vis {
                *v = Visibility::Hidden;
            }
        }
        Some(i) => {
            let c = map.0.cell_by_index(i);
            let centre = map.0.grid.center_world(c.q, c.r);
            let s = map.0.grid.size;
            let t = thick as f64;
            if let Some(mut m) = meshes.get_mut(&h.halo_mesh) {
                *m = build_ring_mesh(centre, s - t * 0.5, s + t);
            }
            if let Some(mut m) = meshes.get_mut(&h.ring_mesh) {
                *m = build_ring_mesh(centre, s - t * 0.5, s + t * 0.25);
            }
            // 輪の外側(縁取り)が内側(色付き)より広くなるよう、縁取りの方が外へ張り出す。
            for mut v in &mut vis {
                *v = Visibility::Inherited;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_texts(
    time: Res<Time>,
    mut frames: ResMut<FrameTimes>,
    map: Option<Res<LoadedHexMap>>,
    selection: Option<Res<HexSelection>>,
    error: Option<Res<HexLoadError>>,
    stats: Option<Res<HexLoadStats>>,
    world: Option<Res<super::world_view::LoadedWorld>>,
    world_err: Option<Res<super::world_view::WorldLoadError>>,
    cam: Query<&Transform, With<HexCamera>>,
    mut info: Query<&mut Text, (With<InfoText>, Without<StatusText>)>,
    mut status: Query<&mut Text, (With<StatusText>, Without<InfoText>)>,
) {
    let ms = time.delta_secs() * 1000.0;
    frames.ema_ms = if frames.ema_ms == 0.0 {
        ms
    } else {
        frames.ema_ms * 0.95 + ms * 0.05
    };
    frames.max_ms = frames.max_ms.max(ms);

    if let (Ok(mut t), Some(err)) = (info.single_mut(), error) {
        t.0 = format!(
            "マップの読み込みに失敗しました\n{}\n\nTab: 画面を閉じて再度開く / Esc: 終了",
            err.0
        );
        return;
    }
    if let (Ok(mut t), Some(map)) = (info.single_mut(), map.as_ref()) {
        t.0 = match selection.and_then(|s| s.0) {
            None => "選択なし\n地図のセルをクリックしてください".to_string(),
            Some(i) => {
                let c = map.0.cell_by_index(i);
                let world_lines = world
                    .as_ref()
                    .map(|w| super::world_view::selection_lines(w, &map.0, c.q, c.r))
                    .unwrap_or_default();
                format!(
                    "選択セル\nq = {}\nr = {}\n地形: {} ({})\n気候: {} ({})\n植生: {} ({}){}",
                    c.q,
                    c.r,
                    render::terrain_label(c.terrain),
                    c.terrain.id(),
                    render::climate_label(c.climate),
                    c.climate.id(),
                    render::vegetation_label(c.vegetation),
                    c.vegetation.id(),
                    world_lines,
                )
            }
        };
    }
    if let (Ok(mut t), Some(stats)) = (status.single_mut(), stats) {
        let scale = cam.single().map(|t| t.scale.x).unwrap_or(1.0);
        t.0 = format!(
            "\n--- 状態 ---\nセル {} / 河川レコード {} / 河川の辺 {}\nチャンク {} / メッシュ {} / マップEntity {}\n読込 {:.0} ms / メッシュ生成 {:.0} ms\nフレーム {:.1} ms (最大 {:.1} ms)\nズーム scale {:.3}",
            stats.cells,
            stats.rivers,
            stats.river_edges,
            stats.chunks,
            stats.mesh_assets,
            stats.spawned_entities,
            stats.load_ms,
            stats.mesh_ms,
            frames.ema_ms,
            frames.max_ms,
            scale
        );
        t.0.push_str(&super::world_view::status_lines(
            world.as_deref(),
            world_err.as_deref(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: (f32, f32, f32, f32) = (0.0, -900.0, 1800.0, 0.0);

    #[test]
    fn cursor_to_world_centre_and_corners() {
        let win = Vec2::new(1000.0, 500.0);
        let cam = Vec2::new(100.0, 50.0);
        assert_eq!(cursor_to_world(Vec2::new(500.0, 250.0), win, cam, 2.0), cam);
        // 左上(0,0)はワールドの左上(Y上向き)
        let tl = cursor_to_world(Vec2::ZERO, win, cam, 2.0);
        assert_eq!(tl, Vec2::new(100.0 - 1000.0, 50.0 + 500.0));
    }

    #[test]
    fn zoom_keeps_point_under_cursor_for_any_window() {
        for win in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(640.0, 900.0),
            Vec2::new(3000.0, 400.0),
        ] {
            let cursor = Vec2::new(win.x * 0.3, win.y * 0.8);
            let cam = Vec2::new(600.0, -300.0);
            let before = cursor_to_world(cursor, win, cam, 1.5);
            let new_cam = zoom_about_cursor(cursor, win, cam, 1.5, 0.4);
            let after = cursor_to_world(cursor, win, new_cam, 0.4);
            assert!(before.distance(after) < 1e-3, "{before} vs {after}");
        }
    }

    #[test]
    fn fit_contains_whole_map_in_free_area() {
        let win = Vec2::new(1280.0, 720.0);
        let (pos, scale) = fit_camera(B, win);
        let tl = cursor_to_world(Vec2::new(LEFT_PANEL_W, 0.0), win, pos, scale);
        let br = cursor_to_world(Vec2::new(win.x - RIGHT_PANEL_W, win.y), win, pos, scale);
        assert!(tl.x <= B.0 && br.x >= B.2, "x {tl} {br}");
        assert!(tl.y >= B.3 && br.y <= B.1, "y {tl} {br}");
    }

    #[test]
    fn narrow_window_falls_back_to_full_window() {
        let (pos, scale) = fit_camera(B, Vec2::new(400.0, 300.0));
        assert!(scale.is_finite() && scale > 0.0 && pos.is_finite());
    }

    #[test]
    fn clamp_limits_scale_and_position_and_recovers_from_nan() {
        let win = Vec2::new(1280.0, 720.0);
        let (_, s) = clamp_view(B, win, Vec2::ZERO, 1e-9);
        assert_eq!(s, MIN_ZOOM_SCALE);
        let (_, s) = clamp_view(B, win, Vec2::ZERO, 1e9);
        assert_eq!(s, max_zoom_scale(B, win));
        let (p, _) = clamp_view(B, win, Vec2::new(1e9, -1e9), 1.0);
        assert_eq!(p, Vec2::new(B.2, B.1));
        let (p, s) = clamp_view(B, win, Vec2::new(f32::NAN, 0.0), f32::INFINITY);
        let (fp, fs) = fit_camera(B, win);
        assert_eq!((p, s), (fp, fs));
    }
}
