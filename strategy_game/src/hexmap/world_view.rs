//! P32-005 (M1): 検証用ワールド設定(ステート配置)をヘックスプレビューへ重ねる表示。
//!
//! `--world-preview`指定時だけ有効(`HexPreviewPlugin.world_preview = Some(..)`)。
//! 読み込み・検証・境界生成は`world::overlay`(GUI非依存)で行い、ここは表示と操作だけを担う。
//! 読み込みに失敗した場合は`WorldLoadError`のみを保持し、所属や境界のデータ・Entityを一切作らない
//! (地理マップの確認はそのまま可能)。確認画面を閉じると、リソースもEntityも全部破棄する
//! (`HexPreviewEntity`の一括削除+`exit_world`)ため、開き直しても重複しない。

use super::data::{Climate, HexMap, Terrain, Vegetation};
use super::render::{self, HexViewMode, boundary_edge_color, build_edge_mesh};
use super::view::{
    HexCamera, HexPreviewEntity, HexPreviewState, HexViewSettings, HexViewport, LeftPanel,
    LoadedHexMap, ModeButton, apply_fit, button_bundle, enter_preview, fit_camera,
    river_zoom_level,
};
use crate::localization::{Locale, TranslationCatalog, translate};
use crate::world::{
    BoundaryKind, CellStatus, MapStateId, WorldPreviewData, cell_status, load_world_preview,
    state_color,
};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::HashMap;

/// 検証用ワールド設定ファイルの場所(`assets/data/world/preview/`)。
#[derive(Resource, Debug, Clone)]
pub struct WorldPreviewPaths {
    pub provinces: String,
    pub states: String,
}

impl WorldPreviewPaths {
    pub fn default_preview() -> Self {
        Self {
            provinces: "assets/data/world/preview/provinces.ron".to_string(),
            states: "assets/data/world/preview/states.ron".to_string(),
        }
    }
}

/// 読み込み・検証に成功したワールド設定(表示用データ+ステート表示名)。
#[derive(Resource)]
pub struct LoadedWorld {
    pub data: WorldPreviewData,
    pub names: HashMap<MapStateId, String>,
}

/// ワールド設定の読み込み失敗(画面表示用)。これがある間は所属・境界の表示データは存在しない。
#[derive(Resource)]
pub struct WorldLoadError(pub String);

type EdgeList = Vec<(i32, i32, u8, [f32; 4])>;

#[derive(Resource)]
struct WorldView {
    borders_enabled: bool,
    focus_requested: bool,
    focus_bounds: Option<(f32, f32, f32, f32)>,
    casing_edges: EdgeList,
    border_edges: EdgeList,
    region_edges: EdgeList,
    casing_mesh: Handle<Mesh>,
    border_mesh: Handle<Mesh>,
    region_mesh: Handle<Mesh>,
    last_level: Option<i32>,
    rebuilds: usize,
}

#[derive(Component)]
struct BorderLayer;
#[derive(Component)]
struct FocusButton;
#[derive(Component)]
struct BordersButton;

pub(super) struct WorldPreviewPlugin {
    pub paths: WorldPreviewPaths,
}

impl Plugin for WorldPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.paths.clone())
            .add_systems(
                OnEnter(HexPreviewState::Open),
                enter_world.after(enter_preview),
            )
            .add_systems(OnExit(HexPreviewState::Open), exit_world)
            .add_systems(
                Update,
                (
                    world_buttons_and_keys,
                    apply_focus.before(apply_fit),
                    sync_borders,
                )
                    .chain()
                    .run_if(in_state(HexPreviewState::Open)),
            );
    }
}

/// 境界線の幅(ワールド単位)。画面上でおよそ2.4pxを保ち、遠景では一辺の半分で頭打ち。
pub fn border_width(size: f32, level: i32) -> f32 {
    let scale = 2f32.powf(level as f32 / 2.0);
    (2.4 * scale).min(0.5 * size)
}

/// 未割当・範囲外との外周線の幅(ステート境界より細い)。
pub fn region_edge_width(size: f32, level: i32) -> f32 {
    let scale = 2f32.powf(level as f32 / 2.0);
    (1.4 * scale).min(0.3 * size)
}

pub fn catalog_names(data: &WorldPreviewData) -> HashMap<MapStateId, String> {
    let catalog = TranslationCatalog::load().ok();
    data.model
        .states()
        .iter()
        .map(|s| {
            let name = match &catalog {
                Some(c) => translate(c, Locale::JaJp, &s.name_key, &[]),
                None => s.name_key.clone(),
            };
            (s.id, name)
        })
        .collect()
}

pub(super) fn enter_world(
    mut commands: Commands,
    map: Option<Res<LoadedHexMap>>,
    paths: Res<WorldPreviewPaths>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    panel: Query<Entity, With<LeftPanel>>,
    viewport: Option<ResMut<HexViewport>>,
) {
    // 地理マップ自体が読めなかった場合は、ワールド設定の読み込みを試みない(原因は地理側のエラー表示)。
    let Some(map) = map else { return };
    let data = match load_world_preview(&paths.provinces, &paths.states, &map.0) {
        Ok(d) => d,
        Err(e) => {
            error!("[WorldPreview] failed to load world preview data: {e}");
            commands.insert_resource(WorldLoadError(e));
            return;
        }
    };
    info!(
        "[WorldPreview] loaded {} provinces / {} states ({} unassigned), {} boundary edges",
        data.model.provinces().len(),
        data.model.states().len(),
        data.report.unassigned_province_count,
        data.boundaries.len()
    );

    // 辺リスト(色つき)。ステート境界は暗い縁取り+明るい本体、外周線は細い単色。
    let mut casing: EdgeList = Vec::new();
    let mut border: EdgeList = Vec::new();
    let mut region: EdgeList = Vec::new();
    let dark = Color::srgb(0.05, 0.05, 0.08).to_linear();
    let dark = [dark.red, dark.green, dark.blue, 1.0];
    for e in &data.boundaries {
        let Some(c) = boundary_edge_color(e.kind) else {
            continue; // Coast: 海岸線は地形の塗りで見えるため描画しない
        };
        let item = (e.q, e.r, e.direction, c);
        if e.kind == BoundaryKind::StateBorder {
            casing.push((e.q, e.r, e.direction, dark));
            border.push(item);
        } else {
            region.push(item);
        }
    }

    let mat = materials.add(ColorMaterial::from(Color::WHITE));
    let casing_mesh = meshes.add(build_edge_mesh(&map.0, &[], 0.0));
    let border_mesh = meshes.add(build_edge_mesh(&map.0, &[], 0.0));
    let region_mesh = meshes.add(build_edge_mesh(&map.0, &[], 0.0));
    for (mesh, z) in [
        (region_mesh.clone(), 1.4),
        (casing_mesh.clone(), 1.65),
        (border_mesh.clone(), 1.7),
    ] {
        commands.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(mat.clone()),
            Transform::from_xyz(0.0, 0.0, z),
            BorderLayer,
            HexPreviewEntity,
        ));
    }

    // 左パネルへ操作を追加(ステートモード・検証地域へ・境界の切替)。
    if let Ok(panel) = panel.single() {
        let state_btn = commands
            .spawn(button_bundle(
                ModeButton(HexViewMode::State),
                "ステート (4)",
            ))
            .id();
        commands.entity(panel).insert_children(4, &[state_btn]);
        let focus = commands
            .spawn(button_bundle(FocusButton, "検証地域へ (F)"))
            .id();
        let borders = commands
            .spawn(button_bundle(BordersButton, "ステート境界 切替 (B)"))
            .id();
        commands.entity(panel).insert_children(8, &[focus, borders]);
    }

    let focus_bounds = data
        .focus_bounds
        .map(|(a, b, c, d)| (a as f32, b as f32, c as f32, d as f32));
    // 起動時は検証地域へ寄る(全体表示は Home / ボタンで戻れる)。
    if let Some(mut v) = viewport {
        v.needs_fit = false;
    }
    let names = catalog_names(&data);
    commands.insert_resource(WorldView {
        borders_enabled: true,
        focus_requested: true,
        focus_bounds,
        casing_edges: casing,
        border_edges: border,
        region_edges: region,
        casing_mesh,
        border_mesh,
        region_mesh,
        last_level: None,
        rebuilds: 0,
    });
    commands.insert_resource(LoadedWorld { data, names });
}

fn exit_world(mut commands: Commands) {
    // Entityは`HexPreviewEntity`の一括削除で消える。リソースはここで確実に破棄する。
    commands.remove_resource::<LoadedWorld>();
    commands.remove_resource::<WorldLoadError>();
    commands.remove_resource::<WorldView>();
}

#[allow(clippy::too_many_arguments)]
fn world_buttons_and_keys(
    keys: Res<ButtonInput<KeyCode>>,
    focus_btn: Query<&Interaction, (Changed<Interaction>, With<FocusButton>)>,
    borders_btn: Query<&Interaction, (Changed<Interaction>, With<BordersButton>)>,
    world: Option<ResMut<WorldView>>,
    settings: Option<ResMut<HexViewSettings>>,
) {
    let Some(mut w) = world else { return };
    if keys.just_pressed(KeyCode::KeyB) || borders_btn.iter().any(|i| *i == Interaction::Pressed) {
        w.borders_enabled = !w.borders_enabled;
    }
    if keys.just_pressed(KeyCode::KeyF) || focus_btn.iter().any(|i| *i == Interaction::Pressed) {
        w.focus_requested = true;
    }
    if let Some(mut s) = settings
        && keys.just_pressed(KeyCode::Digit4)
        && s.mode != HexViewMode::State
    {
        s.mode = HexViewMode::State;
    }
}

fn apply_focus(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    world: Option<ResMut<WorldView>>,
    viewport: Option<Res<HexViewport>>,
) {
    let Some(mut w) = world else { return };
    // 全体表示(Home/ボタン)が要求されている間は、検証地域への移動を取り消して全体表示を優先する。
    if viewport.is_some_and(|v| v.needs_fit) {
        w.focus_requested = false;
    }
    if !w.focus_requested {
        return;
    }
    let (Ok(window), Ok(mut tf), Some(b)) = (windows.single(), cam.single_mut(), w.focus_bounds)
    else {
        return;
    };
    let win = Vec2::new(window.width(), window.height());
    if win.x < 1.0 || win.y < 1.0 {
        return;
    }
    let (pos, scale) = fit_camera(b, win);
    tf.translation.x = pos.x;
    tf.translation.y = pos.y;
    tf.scale = Vec3::splat(scale);
    w.focus_requested = false;
}

/// 境界の表示切替と、ズーム段階が変わった時だけの線幅の作り直し(パンや同一段階のズームでは再構築しない)。
fn sync_borders(
    world: Option<ResMut<WorldView>>,
    map: Option<Res<LoadedHexMap>>,
    cam: Query<&Transform, With<HexCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut layers: Query<&mut Visibility, With<BorderLayer>>,
) {
    let (Some(mut w), Some(map), Ok(tf)) = (world, map, cam.single()) else {
        return;
    };
    let want = if w.borders_enabled {
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
    if w.last_level == Some(level) {
        return;
    }
    w.last_level = Some(level);
    w.rebuilds += 1;
    let size = map.0.grid.size as f32;
    let bw = border_width(size, level) as f64;
    let rw = region_edge_width(size, level) as f64;
    if let Some(mut m) = meshes.get_mut(&w.casing_mesh) {
        *m = build_edge_mesh(&map.0, &w.casing_edges, bw * 1.8);
    }
    if let Some(mut m) = meshes.get_mut(&w.border_mesh) {
        *m = build_edge_mesh(&map.0, &w.border_edges, bw);
    }
    if let Some(mut m) = meshes.get_mut(&w.region_mesh) {
        *m = build_edge_mesh(&map.0, &w.region_edges, rw);
    }
}

// ---------------------------------------------------------------- 表示用テキスト(GUIなしでもテスト可能)

/// 選択セルの所属状態の説明(A〜Dを区別)。先頭に改行を含む。
pub fn selection_lines(world: &LoadedWorld, map: &HexMap, q: i32, r: i32) -> String {
    describe_cell(world, map, q, r)
}

pub fn describe_cell(world: &LoadedWorld, map: &HexMap, q: i32, r: i32) -> String {
    match cell_status(&world.data.model, map, q, r) {
        CellStatus::Assigned { province, state } => {
            let name = world
                .names
                .get(&state)
                .cloned()
                .unwrap_or_else(|| state.to_string());
            format!(
                "\n--- ワールド(検証用) ---\nプロヴィンス: {}\nステート: {} (MapState#{})\n所属: 所属済み{}",
                province.0,
                name,
                state.0,
                state_summary_text(world, state)
            )
        }
        CellStatus::Unassigned { province } => format!(
            "\n--- ワールド(検証用) ---\nプロヴィンス: {}\nステート: なし\n所属: 未割当",
            province.0
        ),
        CellStatus::OutOfScope => {
            "\n--- ワールド(検証用) ---\n所属: 検証範囲外\n(ID表にない陸セル。IDは未発行)"
                .to_string()
        }
        CellStatus::Water => {
            "\n--- ワールド(検証用) ---\n所属: 水域(陸上プロヴィンスの対象外)".to_string()
        }
        CellStatus::NoCell => String::new(),
    }
}

/// 状態欄に足す、ワールド設定の読み込み状況。
pub fn status_lines(world: Option<&LoadedWorld>, err: Option<&WorldLoadError>) -> String {
    if let Some(e) = err {
        return format!(
            "\n--- ワールド設定 ---\n読み込みに失敗(所属・境界は表示しません。地理マップは確認できます)\n{}",
            e.0
        );
    }
    match world {
        Some(w) => {
            let m = &w.data.model;
            format!(
                "\n--- ワールド設定(検証用・部分) ---\nステート {} / プロヴィンス {} / 未割当 {}\n境界の辺 {}(州境 {})\nAuthoring検証: 成功(全域の検証は未完了)",
                m.states().len(),
                m.provinces().len(),
                w.data.report.unassigned_province_count,
                w.data.boundaries.len(),
                w.data
                    .boundaries
                    .iter()
                    .filter(|b| b.kind == BoundaryKind::StateBorder)
                    .count()
            )
        }
        None => String::new(),
    }
}

/// ステートモードの凡例。ステートごとの色と件数、未割当・範囲外・水域。
pub fn state_legend(world: &LoadedWorld, map: &HexMap) -> Vec<(String, [f32; 3])> {
    let mut v = Vec::new();
    for s in world.data.model.states() {
        let name = world
            .names
            .get(&s.id)
            .cloned()
            .unwrap_or_else(|| s.id.to_string());
        v.push((format!("{name} ({})", s.provinces.len()), state_color(s.id)));
    }
    v.push((
        format!("未割当 ({})", world.data.report.unassigned_province_count),
        render::UNASSIGNED_FILL,
    ));
    v.push(("検証範囲外の陸(暗い地形色)".to_string(), [0.2, 0.28, 0.15]));
    v.push(("水域(海・湖)".to_string(), [0.08, 0.20, 0.45]));
    v.push((
        "線: 白=州境 / 橙=未割当との境 / 紫=範囲外との境".to_string(),
        [0.9, 0.9, 0.8],
    ));
    let _ = map;
    v
}

/// 内訳のうち件数が1以上の項目だけを「ラベル件数」で並べる(0件の項目は出さない)。
fn breakdown<T: Copy>(items: &[T], counts: &[usize], label: impl Fn(T) -> String) -> String {
    let parts: Vec<String> = items
        .iter()
        .zip(counts)
        .filter(|(_, n)| **n > 0)
        .map(|(it, n)| format!("{} {}", label(*it), n))
        .collect();
    if parts.is_empty() {
        "なし".to_string()
    } else {
        parts.join(" / ")
    }
}

/// 所属済みステートの地理集計と隣接ステートの確認用テキスト(導出済みデータの表示のみ。
/// 選択のたびに集計を再生成しない)。人口・施設などのシミュレーション値ではない。
pub fn state_summary_text(world: &LoadedWorld, state: MapStateId) -> String {
    let d = &world.data.derived;
    let Some(s) = d.summary(state) else {
        return String::new();
    };
    let name_of = |id: MapStateId| {
        world
            .names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    };
    let mut t = format!(
        "\n--- ステート集計(地理) ---\n{} (MapState#{})\nプロヴィンス数: {}(面積ではない)\n地形: {}\n気候: {}\n植生: {}",
        name_of(state),
        state.0,
        s.province_count,
        breakdown(&Terrain::ALL, &s.terrain, |x| render::terrain_label(x)
            .to_string()),
        breakdown(&Climate::ALL, &s.climate, |x| render::climate_label(x)
            .to_string()),
        breakdown(&Vegetation::ALL, &s.vegetation, |x| {
            render::vegetation_label(x).to_string()
        }),
    );
    let neighbors = d.neighbors_of(state);
    if neighbors.is_empty() {
        t.push_str("\n隣接ステート: 確認できた範囲では なし");
    } else {
        t.push_str("\n隣接ステート:");
        for (other, adj) in neighbors {
            t.push_str(&format!(
                "\n  {} (#{}): 共有辺 {} / 河川辺 {}",
                name_of(other),
                other.0,
                adj.shared_edges,
                adj.shared_river_edges
            ));
        }
    }
    let c = d.completeness();
    if c.is_complete() {
        t.push_str("\n(全域を網羅した完全な結果)");
    } else {
        t.push_str(&format!(
            "\n※検証用の部分データ(coverage {:?}、未割当 {}、範囲外の陸 {})。記録のない隣接は「存在しない」とは限りません",
            c.coverage, c.unassigned_provinces, c.out_of_scope_land_cells
        ));
    }
    t
}
