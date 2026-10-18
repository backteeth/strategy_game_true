//! P32-001: ヘックスマップ確認画面の起動用バイナリ。
//!
//! 起動: `cargo run --bin hex_map_preview` (カレントは`strategy_game/`。`assets/data/hex_atlas_map.json`を読む)
//! 操作: ホイール=ズーム / ドラッグ=パン / 左クリック=選択 / 1,2,3=地形・気候・植生 /
//!       Home=全体表示 / G=境界線 / Tab=画面を閉じる・開く / Esc=終了
//! `--shots <dir>`: 同じコード経路(モード切替・選択・ズーム・全体表示)を自動で順に実行し、
//! 各段階のスクリーンショットを<dir>へ保存して終了する(実マウス操作の代替ではない)。

use bevy::log::{DEFAULT_FILTER, LogPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use strategy_game::hexmap::render::HexViewMode;
use strategy_game::hexmap::view::{
    HexCamera, HexPreviewPlugin, HexSelection, HexViewSettings, HexViewport, LoadedHexMap,
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

#[allow(clippy::too_many_arguments)]
fn shot_script(
    mut commands: Commands,
    dir: Res<ShotDir>,
    mut frame: Local<u32>,
    mut frame_ms: Local<Vec<f32>>,
    time: Res<Time>,
    settings: Option<ResMut<HexViewSettings>>,
    selection: Option<ResMut<HexSelection>>,
    viewport: Option<ResMut<HexViewport>>,
    map: Option<Res<LoadedHexMap>>,
    mut cam: Query<&mut Transform, With<HexCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut settings), Some(mut selection), Some(mut viewport), Some(map)) =
        (settings, selection, viewport, map)
    else {
        return;
    };
    *frame += 1;
    let f = *frame;
    let shot = |commands: &mut Commands, name: &str| {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(dir.0.join(name)));
    };
    if (20..60).contains(&f) {
        frame_ms.push(time.delta_secs() * 1000.0);
    }
    match f {
        30 => shot(&mut commands, "01_overview_terrain.png"),
        60 => {
            let avg = frame_ms.iter().sum::<f32>() / frame_ms.len().max(1) as f32;
            let max = frame_ms.iter().cloned().fold(0.0, f32::max);
            info!(
                "[HexShots] idle full-view frame time over {} frames: avg {avg:.2} ms, max {max:.2} ms",
                frame_ms.len()
            );
            settings.mode = HexViewMode::Climate;
        }
        75 => shot(&mut commands, "02_overview_climate.png"),
        90 => settings.mode = HexViewMode::Vegetation,
        105 => shot(&mut commands, "03_overview_vegetation.png"),
        120 => {
            settings.mode = HexViewMode::Terrain;
            let c = map
                .0
                .cells()
                .iter()
                .find(|c| c.terrain == strategy_game::hexmap::data::Terrain::Mountain && c.q > 150)
                .map(|c| (c.q, c.r))
                .and_then(|(q, r)| map.0.cell(q, r))
                .or_else(|| map.0.cells().iter().find(|c| !c.terrain.is_water()))
                .copied()
                .expect("map has cells");
            selection.0 = map.0.cell_index(c.q, c.r);
            let (x, y) = map.0.grid.center_world(c.q, c.r);
            if let Ok(mut tf) = cam.single_mut() {
                tf.translation.x = x as f32;
                tf.translation.y = y as f32;
                tf.scale = Vec3::splat(0.05);
            }
        }
        140 => shot(&mut commands, "04_selected_zoomed.png"),
        150 => viewport.needs_fit = true,
        170 => shot(&mut commands, "05_after_fit_selected.png"),
        200 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
