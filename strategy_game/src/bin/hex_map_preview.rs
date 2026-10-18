//! P32-001: ヘックスマップ確認画面の起動用バイナリ。
//!
//! 起動: `cargo run --bin hex_map_preview` (カレントは`strategy_game/`。`assets/data/hex_atlas_map.json`を読む)
//! 操作: ホイール=ズーム / ドラッグ=パン / 左クリック=選択 / 1,2,3=地形・気候・植生 /
//!       Home=全体表示 / G=境界線 / R=河川 / Tab=画面を閉じる・開く / Esc=終了
//! `--shots <dir>`: 同じコード経路(モード切替・選択・ズーム・全体表示)を自動で順に実行し、
//! 各段階のスクリーンショットを<dir>へ保存して終了する(実マウス操作の代替ではない)。

use bevy::log::{DEFAULT_FILTER, LogPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use strategy_game::hexmap::render::HexViewMode;
use strategy_game::hexmap::view::{
    HexCamera, HexPreviewEntity, HexPreviewPlugin, HexPreviewState, HexSelection, HexViewSettings,
    HexViewport, LoadedHexMap,
};

#[derive(Resource)]
struct ShotDir(std::path::PathBuf);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shots = args
        .iter()
        .position(|a| a == "--shots")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);

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
    });
    if let Some(dir) = shots {
        std::fs::create_dir_all(&dir).expect("create shot dir");
        app.insert_resource(ShotDir(dir))
            .add_systems(Update, shot_script);
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
