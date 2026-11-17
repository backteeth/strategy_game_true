//! P32-001: ヘックスマップ確認画面の起動用バイナリ。
//!
//! 起動: `cargo run --bin hex_map_preview` (カレントは`strategy_game/`。`assets/data/hex_atlas_map.json`を読む)
//! `--world-preview`: 検証用ワールド設定(P32-005。正式な世界設定ではない)を重ねて表示する。
//!       追加操作: 4=ステート塗り / B=ステート境界の切替 / F=検証地域へ移動(Home=全体表示)
//! 操作: ホイール=ズーム / ドラッグ=パン / 左クリック=選択 / 1,2,3=地形・気候・植生 /
//!       Home=全体表示 / G=境界線 / R=河川 / Tab=画面を閉じる・開く / Esc=終了
//! `--shots <dir>`: 同じコード経路(モード切替・選択・ズーム・全体表示)を自動で順に実行し、
//! 各段階のスクリーンショットを<dir>へ保存して終了する(実マウス操作の代替ではない)。

use bevy::log::{DEFAULT_FILTER, LogPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use strategy_game::hexmap::division_view::{DivisionCommand, DivisionSessionRes, DivisionViewConfig, DivisionViewPlugin};
use strategy_game::hexmap::render::HexViewMode;
use strategy_game::hexmap::view::{
    HexCamera, HexPreviewEntity, HexPreviewPlugin, HexPreviewState, HexSelection, HexViewSettings,
    HexViewport, LoadedHexMap,
};
use strategy_game::hexmap::world_view::{LoadedWorld, WorldPreviewPaths};
use strategy_game::world::{BoundaryKind, CellStatus, cell_status};

#[derive(Resource)]
struct ShotDir(std::path::PathBuf);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shots = args
        .iter()
        .position(|a| a == "--shots")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

    // --world-preview: 検証用ワールド設定(assets/data/world/preview/)を重ねる。
    // --world-provinces <path> / --world-states <path> で別ファイルを指定できる(読み込み失敗の確認用)。
    let arg_after = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let world_preview = args.iter().any(|a| a == "--world-preview").then(|| {
        let mut p = WorldPreviewPaths::default_preview();
        if let Some(v) = arg_after("--world-provinces") {
            p.provinces = v;
        }
        if let Some(v) = arg_after("--world-states") {
            p.states = v;
        }
        p
    });
    // P32-039: --division-scenario <RON>。検証用の師団シナリオを開く(--world-preview を含意する)。値が無い場合は起動しない。
    let division_scenario = args.iter().position(|a| a == "--division-scenario").map(|i| {
        args.get(i + 1).filter(|v| !v.starts_with("--")).cloned().unwrap_or_else(|| {
            eprintln!("hex_map_preview: --division-scenario needs a path to a division scenario RON");
            std::process::exit(2)
        })
    });
    let world_preview = match (world_preview, division_scenario.is_some()) {
        (None, true) => Some(WorldPreviewPaths::default_preview()),
        (w, _) => w,
    };
    let with_world = world_preview.is_some();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(LogPlugin {
                filter: format!("{DEFAULT_FILTER}icu_provider=error,"),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Hex Map Preview (P32-001)".to_string(),
                    resolution: (1600u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_systems(
        PreStartup,
        strategy_game::localization::install_japanese_capable_font,
    )
    .add_plugins(HexPreviewPlugin {
        open_at_start: true,
        world_preview,
    });
    if let Some(path) = division_scenario.clone() {
        app.add_plugins(DivisionViewPlugin { config: DivisionViewConfig { scenario: path } });
    }
    if let Some(dir) = shots {
        std::fs::create_dir_all(&dir).expect("create shot dir");
        app.insert_resource(ShotDir(dir));
        if division_scenario.is_some() {
            app.add_systems(Update, division_shot_script);
        } else if with_world {
            app.add_systems(Update, world_shot_script);
        } else {
            app.add_systems(Update, shot_script);
        }
    }
    app.run();
}

#[derive(Default)]
struct Stats {
    idle: Vec<f32>,
    moving: Vec<f32>,
}

fn summarize(label: &str, v: &[f32]) {
    let avg = v.iter().sum::<f32>() / v.len().max(1) as f32;
    let max = v.iter().cloned().fold(0.0, f32::max);
    info!(
        "[HexShots] {label}: {} frames avg {avg:.2} ms max {max:.2} ms",
        v.len()
    );
}

/// 自動スクリプト。実マウス操作の代替ではなく、同じ内部経路を順に実行する。
#[allow(clippy::too_many_arguments)]
fn shot_script(
    mut commands: Commands,
    dir: Res<ShotDir>,
    mut frame: Local<u32>,
    mut stats: Local<Stats>,
    time: Res<Time>,
    settings: Option<ResMut<HexViewSettings>>,
    selection: Option<ResMut<HexSelection>>,
    viewport: Option<ResMut<HexViewport>>,
    map: Option<Res<LoadedHexMap>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    preview_entities: Query<Entity, With<HexPreviewEntity>>,
    mut next: ResMut<NextState<HexPreviewState>>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    let f = *frame;
    // 閉じている間(f 260〜280)は再入場の確認だけを行う。
    if f == 260 {
        info!(
            "[HexShots] entities before close: {}",
            preview_entities.iter().count()
        );
        next.set(HexPreviewState::Closed);
        return;
    }
    if f == 275 {
        info!(
            "[HexShots] entities while closed: {}",
            preview_entities.iter().count()
        );
        next.set(HexPreviewState::Open);
        return;
    }
    let (Some(mut settings), Some(mut selection), Some(mut viewport), Some(map)) =
        (settings, selection, viewport, map)
    else {
        return;
    };
    let shot = |commands: &mut Commands, name: &str| {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(dir.0.join(name)));
    };
    let ms = time.delta_secs() * 1000.0;
    if (20..60).contains(&f) {
        stats.idle.push(ms);
    }
    // 河川のある、両側セルが存在する辺(最初のもの)
    let edge = map
        .0
        .river_edges()
        .iter()
        .find(|e| e.neighbor_exists)
        .copied()
        .expect("map has rivers");
    let focus = |tf: &mut Transform, scale: f32| {
        let (a, b) = strategy_game::hexmap::geometry::river_edge_world(
            &map.0.grid,
            edge.q,
            edge.r,
            edge.direction,
        );
        tf.translation.x = ((a.0 + b.0) / 2.0) as f32;
        tf.translation.y = ((a.1 + b.1) / 2.0) as f32;
        tf.scale = Vec3::splat(scale);
    };
    match f {
        30 => shot(&mut commands, "01_overview_terrain_rivers.png"),
        60 => {
            summarize("idle full view", &stats.idle);
            settings.mode = HexViewMode::Climate;
        }
        75 => shot(&mut commands, "02_overview_climate_rivers.png"),
        90 => settings.mode = HexViewMode::Vegetation,
        105 => shot(&mut commands, "03_overview_vegetation_rivers.png"),
        120 => {
            settings.mode = HexViewMode::Terrain;
            selection.0 = map.0.cell_index(edge.q, edge.r);
            if let Ok(mut tf) = cam.single_mut() {
                focus(&mut tf, 0.05);
            }
        }
        140 => shot(&mut commands, "04_river_closeup_selected.png"),
        145 => {
            if let Ok(mut tf) = cam.single_mut() {
                focus(&mut tf, 0.2);
            }
        }
        160 => shot(&mut commands, "05_river_midzoom.png"),
        165 => settings.rivers_enabled = false,
        180 => shot(&mut commands, "06_rivers_off_midzoom.png"),
        185 => {
            settings.rivers_enabled = true;
            viewport.needs_fit = true;
        }
        200 => shot(&mut commands, "07_after_fit_rivers.png"),
        // パン・ズーム中のフレーム時間: 210〜250でスケールとカメラ位置を動かす
        210..=250 => {
            if let Ok(mut tf) = cam.single_mut() {
                let t = (f - 210) as f32 / 40.0;
                let s = 1.0 * (0.05f32 / 1.0).powf((t * std::f32::consts::PI).sin());
                tf.scale = Vec3::splat(s.max(0.05));
                tf.translation.x += 6.0 * s;
                stats.moving.push(ms);
            }
        }
        251 => summarize("pan/zoom moving", &stats.moving),
        285 => shot(&mut commands, "08_after_reentry.png"),
        290 => {
            info!(
                "[HexShots] entities after reopen: {}",
                preview_entities.iter().count()
            );
        }
        300 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// `--world-preview --shots`用の自動スクリプト(実マウス操作の代替ではない)。
#[allow(clippy::too_many_arguments)]
fn world_shot_script(
    mut commands: Commands,
    dir: Res<ShotDir>,
    mut frame: Local<u32>,
    settings: Option<ResMut<HexViewSettings>>,
    selection: Option<ResMut<HexSelection>>,
    viewport: Option<ResMut<HexViewport>>,
    map: Option<Res<LoadedHexMap>>,
    world: Option<Res<LoadedWorld>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    preview_entities: Query<Entity, With<HexPreviewEntity>>,
    mut next: ResMut<NextState<HexPreviewState>>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    let f = *frame;
    if f == 250 {
        info!(
            "[WorldShots] entities before close: {}",
            preview_entities.iter().count()
        );
        next.set(HexPreviewState::Closed);
        return;
    }
    if f == 262 {
        info!(
            "[WorldShots] entities while closed: {}",
            preview_entities.iter().count()
        );
        next.set(HexPreviewState::Open);
        return;
    }
    if f == 285 {
        info!(
            "[WorldShots] entities after reopen: {}",
            preview_entities.iter().count()
        );
    }
    let (Some(mut settings), Some(mut selection), Some(mut viewport), Some(map)) =
        (settings, selection, viewport, map)
    else {
        return;
    };
    let shot = |commands: &mut Commands, name: &str| {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(dir.0.join(name)));
    };
    let Some(world) = world else {
        // ワールド設定の読み込みに失敗した場合(エラー表示の確認)
        if f == 40 {
            shot(&mut commands, "w_error_geography_only.png");
        }
        if f == 60 {
            exit.write(AppExit::Success);
        }
        return;
    };
    let m = &world.data.model;
    let status = |q: i32, r: i32| cell_status(m, &map.0, q, r);
    let pick_cell = |want: &dyn Fn(CellStatus) -> bool| {
        map.0
            .cells()
            .iter()
            .filter(|c| (c.q - 83).abs() <= 8 && (c.r - 81).abs() <= 8)
            .find(|c| want(status(c.q, c.r)))
            .copied()
    };
    let focus_cell = |tf: &mut Transform, q: i32, r: i32, scale: f32| {
        let (x, y) = map.0.grid.center_world(q, r);
        tf.translation.x = x as f32;
        tf.translation.y = y as f32;
        tf.scale = Vec3::splat(scale);
    };
    let border = world
        .data
        .boundaries
        .iter()
        .find(|b| b.kind == BoundaryKind::StateBorder)
        .copied();
    match f {
        40 => shot(&mut commands, "01_world_focus_terrain.png"),
        50 => settings.mode = HexViewMode::State,
        70 => shot(&mut commands, "02_world_state_mode.png"),
        75 => {
            if let Some(b) = border {
                selection.0 = map.0.cell_index(b.q, b.r);
            }
        }
        95 => shot(&mut commands, "03_selected_assigned.png"),
        100 => {
            if let Some(c) = pick_cell(&|s| matches!(s, CellStatus::Unassigned { .. })) {
                selection.0 = map.0.cell_index(c.q, c.r);
            }
        }
        120 => shot(&mut commands, "04_selected_unassigned.png"),
        125 => {
            if let Some(c) = pick_cell(&|s| matches!(s, CellStatus::OutOfScope)) {
                selection.0 = map.0.cell_index(c.q, c.r);
            }
        }
        145 => shot(&mut commands, "05_selected_out_of_scope.png"),
        150 => {
            if let Some(c) = pick_cell(&|s| matches!(s, CellStatus::Water)) {
                selection.0 = map.0.cell_index(c.q, c.r);
            }
        }
        170 => shot(&mut commands, "06_selected_water.png"),
        175 => settings.mode = HexViewMode::Climate,
        190 => shot(&mut commands, "07_borders_over_climate.png"),
        195 => {
            settings.mode = HexViewMode::Terrain;
            if let Some(b) = border {
                selection.0 = map.0.cell_index(b.q, b.r);
                if let Ok(mut tf) = cam.single_mut() {
                    focus_cell(&mut tf, b.q, b.r, 0.05);
                }
            }
        }
        215 => shot(
            &mut commands,
            "08_border_closeup_with_river_and_selection.png",
        ),
        220 => viewport.needs_fit = true,
        240 => shot(&mut commands, "09_after_full_map_return.png"),
        290 => shot(&mut commands, "10_after_reopen.png"),
        // P32-006: ステート集計の確認(別ステートへ移ると集計が切り替わる)
        295 => {
            settings.mode = HexViewMode::State;
            selection.0 = map.0.cell_index(86, 78); // MapState#21
            viewport.needs_fit = true;
        }
        320 => shot(&mut commands, "11_state_summary_beta.png"),
        325 => selection.0 = map.0.cell_index(80, 83), // MapState#20
        345 => shot(&mut commands, "12_state_summary_alpha.png"),
        350 => selection.0 = map.0.cell_index(83, 78), // MapState#22
        370 => shot(&mut commands, "13_state_summary_gamma.png"),
        375 => {
            // 未割当を選ぶと、前の集計が残らない
            if let Some(c) = pick_cell(&|s| matches!(s, CellStatus::Unassigned { .. })) {
                selection.0 = map.0.cell_index(c.q, c.r);
            }
        }
        395 => shot(&mut commands, "14_unassigned_no_stale_summary.png"),
        400 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// `--division-scenario --shots`用の自動スクリプト(実マウス操作の代替ではない。同じ `DivisionCommand` の経路を使う)。
/// 移動シナリオ(ファイル名に "battle" を含まない): 師団1を選び、18309→18308 へ命令して 5 日進める(4日目まで 18309、5日目に 18308)。
/// 戦闘シナリオ("battle" を含む): 同じ 18311 にいる師団1・4 の選択、師団2 の 17694 への攻撃(戦闘→占領)、表示モード切替とズーム後の確認。
/// 領土移転シナリオ("territory" を含む): 国家2 の師団でステート22 を併合(国家1 の首都移転)→ 国家1 の師団でステート21 を併合 → ステート22 を併合して国家2 が消滅。
/// いずれも最後に Tab で閉じて開き直し、初期配置へ戻ることを確認する。各段階の師団の位置は `[DivisionShots]` のログに出す。
#[allow(clippy::too_many_arguments)]
fn division_shot_script(
    mut commands: Commands,
    dir: Res<ShotDir>,
    config: Res<DivisionViewConfig>,
    mut frame: Local<u32>,
    settings: Option<ResMut<HexViewSettings>>,
    selection: Option<ResMut<HexSelection>>,
    session: Option<Res<DivisionSessionRes>>,
    world: Option<Res<LoadedWorld>>,
    map: Option<Res<LoadedHexMap>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    mut out: MessageWriter<DivisionCommand>,
    mut next: ResMut<NextState<HexPreviewState>>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    let f = *frame;
    let shot = |commands: &mut Commands, name: &str| {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(dir.0.join(name)));
    };
    let battle = config.scenario.contains("battle");
    let territory = config.scenario.contains("territory");
    let log_state = |tag: &str| {
        if let Some(s) = &session {
            let m = s.0.military();
            let parts: Vec<String> = m
                .divisions()
                .map(|d| format!("{}@{}:{}{}", d.id.0, d.province.0, division_status(d.status), d.destination.map(|p| format!("->{}", p.0)).unwrap_or_default()))
                .collect();
            info!("[DivisionShots] {tag}: day {} selected {:?} divisions {}", s.0.day(), s.0.selected().map(|i| i.0), parts.join(" "));
        } else {
            info!("[DivisionShots] {tag}: no session");
        }
    };
    if f == 300 {
        next.set(HexPreviewState::Closed);
        return;
    }
    if f == 310 {
        next.set(HexPreviewState::Open);
        return;
    }
    if f == 340 {
        log_state("after reopen (must be the initial placement, no selection)");
        shot(&mut commands, "d_after_reopen.png");
    }
    if f == 360 {
        exit.write(AppExit::Success);
        return;
    }
    let (Some(mut settings), Some(mut selection), Some(world), Some(map)) = (settings, selection, world, map) else {
        if f == 40 {
            shot(&mut commands, "d_error.png");
        }
        if f == 60 {
            exit.write(AppExit::Success);
        }
        return;
    };
    if session.is_none() {
        // 起動失敗(エラー表示の確認)
        if f == 40 {
            shot(&mut commands, "d_error.png");
        }
        if f == 60 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let cell_of = |p: u32| world.data.model.province(strategy_game::world::ProvinceId(p)).map(|d| (d.cell.q, d.cell.r));
    let select_cell = |sel: &mut HexSelection, p: u32| {
        if let Some((q, r)) = cell_of(p) {
            sel.0 = map.0.cell_index(q, r);
        }
    };
    let focus = |tf: &mut Transform, p: u32, scale: f32| {
        if let Some((q, r)) = cell_of(p) {
            let (x, y) = map.0.grid.center_world(q, r);
            tf.translation.x = x as f32;
            tf.translation.y = y as f32;
            tf.scale = Vec3::splat(scale);
        }
    };
    use strategy_game::world::MapDivisionId as D;
    if territory {
        // 領土移転(P32-041): 選択した師団の国が、選択セルのステートを併合する(同じ DivisionCommand の経路)。
        match f {
            40 => {
                log_state("initial");
                shot(&mut commands, "t1_initial.png");
            }
            45 => {
                out.write(DivisionCommand::Select(D(6)));
                select_cell(&mut selection, 17266);
                settings.mode = HexViewMode::State;
                if let Ok(mut tf) = cam.single_mut() {
                    focus(&mut tf, 17485, 0.06);
                }
            }
            60 => {
                log_state("division 6 (country 2) selected, a cell of state 22 (country 1's, capital 17266) selected");
                shot(&mut commands, "t2_before_cession_of_state22.png");
            }
            65 => {
                out.write(DivisionCommand::TransferSelectedCellState);
            }
            85 => {
                log_state("after state 22 -> country 2 (country 1's capital moves, 17484 occupation by country 3 resolved, division 17 returns)");
                shot(&mut commands, "t3_state22_to_country2.png");
            }
            90 => {
                out.write(DivisionCommand::Select(D(1)));
                select_cell(&mut selection, 17269);
            }
            95 => {
                out.write(DivisionCommand::TransferSelectedCellState);
            }
            115 => {
                log_state("after state 21 -> country 1 (country 2 still owns state 22: not eliminated)");
                shot(&mut commands, "t4_state21_to_country1.png");
            }
            120 => {
                out.write(DivisionCommand::Select(D(1)));
                select_cell(&mut selection, 17266);
            }
            125 => {
                out.write(DivisionCommand::TransferSelectedCellState);
            }
            145 => {
                log_state("after state 22 -> country 1 (country 2 lost its last state: eliminated)");
                shot(&mut commands, "t5_country2_eliminated.png");
            }
            _ => {}
        }
    } else if !battle {
        match f {
            40 => {
                log_state("initial");
                shot(&mut commands, "d1_initial.png");
            }
            45 => {
                out.write(DivisionCommand::Select(D(1)));
                select_cell(&mut selection, 18308);
            }
            60 => {
                log_state("selected division 1, destination cell 18308 selected");
                shot(&mut commands, "d2_selected.png");
            }
            65 => {
                out.write(DivisionCommand::OrderToSelectedCell);
            }
            80 => {
                log_state("order accepted");
                shot(&mut commands, "d3_ordered.png");
            }
            85 | 90 | 95 | 100 => {
                out.write(DivisionCommand::AdvanceDay);
            }
            110 => {
                log_state("after 4 days (division 1 must still be at 18309)");
                shot(&mut commands, "d4_day4.png");
            }
            115 => {
                out.write(DivisionCommand::AdvanceDay);
            }
            130 => {
                log_state("after 5 days (division 1 must be at 18308)");
                shot(&mut commands, "d5_day5.png");
            }
            135 => {
                // 表示モード切替・ズーム・パンの後も、マーカーと選択が同じセルに乗る
                settings.mode = HexViewMode::State;
                if let Ok(mut tf) = cam.single_mut() {
                    focus(&mut tf, 18308, 0.05);
                }
            }
            155 => {
                log_state("after mode switch and zoom");
                shot(&mut commands, "d6_zoom_state_mode.png");
            }
            160 => {
                settings.mode = HexViewMode::Terrain;
                if let Ok(mut tf) = cam.single_mut() {
                    focus(&mut tf, 17266, 0.12);
                }
                out.write(DivisionCommand::Select(D(2)));
            }
            180 => {
                log_state("division 2 selected after pan");
                shot(&mut commands, "d7_pan_division2.png");
            }
            _ => {}
        }
    } else {
        match f {
            40 => {
                log_state("initial");
                shot(&mut commands, "b1_initial.png");
            }
            45 => {
                // 同じプロヴィンス(18311)に師団1と4がいる: 件数表示と、選択欄からの選択
                select_cell(&mut selection, 18311);
                if let Ok(mut tf) = cam.single_mut() {
                    focus(&mut tf, 18311, 0.06);
                }
            }
            60 => {
                log_state("two divisions at 18311");
                shot(&mut commands, "b2_two_divisions_one_province.png");
            }
            65 => {
                out.write(DivisionCommand::Select(D(4)));
            }
            80 => {
                log_state("division 4 picked");
                shot(&mut commands, "b3_division4_selected.png");
            }
            85 => {
                out.write(DivisionCommand::Select(D(2)));
                select_cell(&mut selection, 17694);
                if let Ok(mut tf) = cam.single_mut() {
                    focus(&mut tf, 17694, 0.08);
                }
            }
            95 => {
                out.write(DivisionCommand::OrderToSelectedCell);
            }
            105 => {
                log_state("division 2 ordered to 17694");
                shot(&mut commands, "b4_ordered.png");
            }
            110 | 112 | 114 | 116 | 118 => {
                out.write(DivisionCommand::AdvanceDay);
            }
            125 => {
                log_state("after 5 days (battle expected)");
                shot(&mut commands, "b5_day5_battle.png");
            }
            130 | 132 | 134 | 136 | 138 => {
                out.write(DivisionCommand::AdvanceDay);
            }
            150 => {
                log_state("after 10 days (occupation expected)");
                shot(&mut commands, "b6_day10_occupied.png");
            }
            155 => {
                settings.mode = HexViewMode::State;
            }
            175 => {
                log_state("state mode after occupation");
                shot(&mut commands, "b7_state_mode_after_occupation.png");
            }
            _ => {}
        }
    }
}

fn division_status(s: strategy_game::world::MapDivisionStatus) -> &'static str {
    match s {
        strategy_game::world::MapDivisionStatus::Idle => "idle",
        strategy_game::world::MapDivisionStatus::Moving => "moving",
        strategy_game::world::MapDivisionStatus::Fighting => "fighting",
    }
}
