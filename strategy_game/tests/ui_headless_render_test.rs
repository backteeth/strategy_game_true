//! P20-007: Headless環境における本番UiPlugin描画の実観測・自動検証
//!
//! 本番 `main.rs` と同一のプラグイン構成（`UiPlugin` を含む全ゲームプラグイン）を
//! Windowを一切生成しないHeadless構成で起動し、固定解像度のoffscreen `RenderTarget::Image`
//! へ本番のUI Cameraを接続して、RenderGraphを実フレーム分実行し、GPUからピクセルを
//! readbackしてPNGとして保存した上で、ピクセル内容を自動assertする。
//!
//! 実行方法:
//!   cargo test --test ui_headless_render_test -- --nocapture
//!
//! Adapter/Backendが利用できない場合はpanicしてテストを失敗させる(スキップしない)。

use bevy::app::PluginsState;
use bevy::asset::AssetId;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{
    RenderAdapterInfo, RenderContext, RenderDevice, RenderGraph, RenderQueue,
};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, Render, RenderApp, RenderSystems};
use bevy::ui::{IsDefaultUiCamera, UiSystems};
use bevy::window::{ExitCondition, WindowPlugin};
use bevy::winit::WinitPlugin;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::building::BuildingPlugin;
use strategy_game::country::CountryPlugin;
use strategy_game::debug::DebugPlugin;
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::logistics::LogisticsPlugin;
use strategy_game::map::MapPlugin;
use strategy_game::map::camera::GameCamera;
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::state::StatePlugin;
use strategy_game::ui::UiPlugin;
use strategy_game::ui::country_selection::{
    CountrySelectButton, CountrySelectionRoot, StartGameButton,
};
use strategy_game::ui::top_bar::TopBarRoot;
use strategy_game::war::WarPlugin;

/// offscreen RenderTargetの固定解像度。640x480はwgpuのCOPY_BYTES_PER_ROW_ALIGNMENT(256)に
/// 対して行パディングが発生しない幅(640*4=2560=256*10)を選び、readback検証を単純化する。
const CAPTURE_WIDTH: u32 = 640;
const CAPTURE_HEIGHT: u32 = 480;

/// 背景色との差分をどこから「UI由来の非背景ピクセル」とみなすかの許容誤差(RGBA各chの最大差)。
const BG_TOLERANCE: i16 = 16;

/// 各チェックポイントで最低限期待する非背景ピクセル数。
const MIN_NON_BACKGROUND_PIXELS: usize = 300;

/// 状態変更前後で最低限期待する差分ピクセル数。
const MIN_DIFF_PIXELS: usize = 50;

const WARMUP_FRAMES: usize = 30;
const SETTLE_FRAMES: usize = 20;

// ─────────────────────────────────────────────────────────────────────────
// Headless offscreen readback インフラ
// (Bevy公式 examples/app/headless_renderer.rs のImageCopyPluginパターンを踏襲)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<Vec<u8>>);

#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

#[derive(Resource, Default)]
struct LatestFrame(Option<Vec<u8>>);

#[derive(Resource, Clone)]
struct CaptureTarget {
    image: Handle<Image>,
    width: u32,
    height: u32,
}

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    src_image: Handle<Image>,
}

impl ImageCopier {
    fn new(src_image: Handle<Image>, size: Extent3d, render_device: &RenderDevice) -> ImageCopier {
        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(size.width as usize * 4);
        let cpu_buffer = render_device.create_buffer(&BufferDescriptor {
            label: Some("p20_007_headless_ui_readback_buffer"),
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ImageCopier {
            buffer: cpu_buffer,
            src_image,
        }
    }
}

#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ImageCopiers(Vec<ImageCopier>);

struct ImageCopyPlugin;

impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();

        app.insert_resource(MainWorldReceiver(r))
            .init_resource::<LatestFrame>()
            .add_systems(PostUpdate, drain_capture_channel);

        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .insert_resource(RenderWorldSender(s))
            .add_systems(ExtractSchedule, image_copy_extract)
            .add_systems(
                Render,
                receive_image_from_buffer.after(RenderSystems::Render),
            )
            .add_systems(RenderGraph, image_copy_driver);
    }
}

fn image_copy_extract(mut commands: Commands, image_copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(
        image_copiers.iter().cloned().collect::<Vec<ImageCopier>>(),
    ));
}

fn image_copy_driver(
    render_context: RenderContext,
    image_copiers: Res<ImageCopiers>,
    render_queue: Res<RenderQueue>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    for image_copier in image_copiers.iter() {
        let Some(src_image) = gpu_images.get(&image_copier.src_image) else {
            continue;
        };

        let mut encoder = render_context
            .render_device()
            .create_command_encoder(&CommandEncoderDescriptor::default());

        let block_dimensions = src_image.texture_descriptor.format.block_dimensions();
        let block_size = src_image
            .texture_descriptor
            .format
            .block_copy_size(None)
            .expect("[P20-007] readback texture format must have a known block size");

        let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
            (src_image.texture_descriptor.size.width as usize / block_dimensions.0 as usize)
                * block_size as usize,
        );

        encoder.copy_texture_to_buffer(
            src_image.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &image_copier.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(
                        std::num::NonZero::<u32>::new(padded_bytes_per_row as u32)
                            .expect("[P20-007] padded_bytes_per_row must be non-zero")
                            .into(),
                    ),
                    rows_per_image: None,
                },
            },
            src_image.texture_descriptor.size,
        );

        render_queue.submit(std::iter::once(encoder.finish()));
    }
}

fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for image_copier in image_copiers.iter() {
        let buffer_slice = image_copier.buffer.slice(..);

        let (s, r) = crossbeam_channel::bounded(1);
        buffer_slice.map_async(MapMode::Read, move |result| match result {
            Ok(()) => s
                .send(())
                .expect("[P20-007] Failed to send map_async completion"),
            Err(err) => panic!("[P20-007] readback: Failed to map GPU buffer for CPU read: {err}"),
        });

        render_device
            .poll(PollType::wait_indefinitely())
            .expect("[P20-007] readback: Failed to poll RenderDevice while waiting for map_async");

        r.recv()
            .expect("[P20-007] readback: Failed to receive map_async completion signal");

        let data = buffer_slice.get_mapped_range().to_vec();
        let _ = sender.send(data);
        image_copier.buffer.unmap();
    }
}

fn drain_capture_channel(receiver: Res<MainWorldReceiver>, mut latest: ResMut<LatestFrame>) {
    while let Ok(data) = receiver.try_recv() {
        latest.0 = Some(data);
    }
}

/// 本番の `GameCamera`(map::camera::setup_camera が Startup で生成)をoffscreen画像へ接続し、
/// UIのデフォルトカメラとしてマークする。本番のカメラ生成システム自体は変更しない。
fn setup_headless_capture(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    camera_q: Query<Entity, With<GameCamera>>,
) {
    let size = Extent3d {
        width: CAPTURE_WIDTH,
        height: CAPTURE_HEIGHT,
        ..default()
    };

    let mut render_target_image =
        Image::new_target_texture(size.width, size.height, TextureFormat::Rgba8UnormSrgb, None);
    render_target_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let render_target_handle = images.add(render_target_image);

    commands.spawn(ImageCopier::new(
        render_target_handle.clone(),
        size,
        &render_device,
    ));

    commands.insert_resource(CaptureTarget {
        image: render_target_handle.clone(),
        width: size.width,
        height: size.height,
    });

    let camera_entity = camera_q.single().expect(
        "[P20-007] production GameCamera entity (map::camera::setup_camera) must exist before headless capture setup runs",
    );
    commands.entity(camera_entity).insert((
        RenderTarget::Image(render_target_handle.into()),
        IsDefaultUiCamera,
    ));
}

// ─────────────────────────────────────────────────────────────────────────
// App構築: main.rs と同一のプラグイン構成 + Headless化
// ─────────────────────────────────────────────────────────────────────────

fn build_headless_app() -> App {
    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            // WinitPluginはWindowを生成しようとして環境によってはpanicするため無効化。
            .disable::<WinitPlugin>()
            // Pipelined renderingを無効化し、Extract/Renderを同一フレーム内で同期実行させ、
            // テストの決定論性を確保する。
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );

    // main.rs と同一順序・同一プラグイン構成(本番UI登録経路)。
    app.add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(PopulationPlugin)
        .add_plugins(LogisticsPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MapPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(DebugPlugin);

    app.add_plugins(ImageCopyPlugin);
    app.add_plugins(InputInjectionPlugin);
    app.add_systems(PostStartup, setup_headless_capture);

    app
}

/// `App::run()` は self を runner へmoveしてしまい呼び出し後に世界を検査できないため、
/// 代わりに `run_once` ランナーと同じ初期化列(plugin readyを待つ→finish→cleanup)を
/// 手動で再現し、その後 `app.update()` を明示的に繰り返す。
fn initialize_app(app: &mut App) {
    let start = std::time::Instant::now();
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
        assert!(
            start.elapsed().as_secs() < 60,
            "[P20-007] Timed out waiting for plugins to finish adding (>60s). Possible Adapter/Backend initialization stall."
        );
    }
    app.finish();
    app.cleanup();
}

// ─────────────────────────────────────────────────────────────────────────
// フレーム解析・PNG保存ヘルパー
// ─────────────────────────────────────────────────────────────────────────

fn unpad_rgba(raw: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(row_bytes);
    if padded_bytes_per_row == row_bytes {
        raw[..row_bytes * height as usize].to_vec()
    } else {
        raw.chunks(padded_bytes_per_row)
            .take(height as usize)
            .flat_map(|row| row[..row_bytes.min(row.len())].to_vec())
            .collect()
    }
}

struct FrameAnalysis {
    non_background_count: usize,
    unique_color_count: usize,
    bbox: Option<(u32, u32, u32, u32)>,
    background_ref: [u8; 4],
}

fn analyze_frame(rgba: &[u8], width: u32, height: u32) -> FrameAnalysis {
    let background_ref = [rgba[0], rgba[1], rgba[2], rgba[3]];
    let mut non_background_count = 0usize;
    let mut colors: HashSet<[u8; 4]> = HashSet::new();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let px = [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]];
            colors.insert(px);

            let diff = px
                .iter()
                .zip(background_ref.iter())
                .map(|(a, b)| (*a as i16 - *b as i16).abs())
                .max()
                .unwrap_or(0);

            if diff > BG_TOLERANCE {
                non_background_count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    let bbox = if non_background_count > 0 {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    };

    FrameAnalysis {
        non_background_count,
        unique_color_count: colors.len(),
        bbox,
        background_ref,
    }
}

fn diff_pixel_count(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(
        a.len(),
        b.len(),
        "[P20-007] frame buffers must be same size to diff"
    );
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(pa, pb)| pa != pb)
        .count()
}

fn save_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            panic!("[P20-007] Failed to create PNG output dir {parent:?}: {e}")
        });
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)
        .unwrap_or_else(|e| panic!("[P20-007] Failed to save PNG {path:?}: {e}"));
}

fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("verification_logs/p20-007/screenshots")
}

/// 現時点でReadback済みの最新フレームを取得し、幅・高さ整合性をassertする。
fn capture_and_verify_frame(app: &App, label: &str) -> (Vec<u8>, u32, u32) {
    let target = app.world().resource::<CaptureTarget>().clone();
    let latest = app.world().resource::<LatestFrame>();
    let raw = latest.0.clone().unwrap_or_else(|| {
        panic!(
            "[P20-007][FAIL:readback] checkpoint '{label}': LatestFrame is None. \
             RenderGraphのimage_copy_driver / receive_image_from_bufferが実行されなかったか、\
             GPU Adapterからのreadbackが完了していません。"
        )
    });

    let rgba = unpad_rgba(&raw, target.width, target.height);
    let expected_len = (target.width * target.height * 4) as usize;
    assert_eq!(
        rgba.len(),
        expected_len,
        "[P20-007][FAIL:readback] checkpoint '{label}': unpadded frame byte length mismatch (expected {expected_len}, got {})",
        rgba.len()
    );

    (rgba, target.width, target.height)
}

// ─────────────────────────────────────────────────────────────────────────
// UI操作ヘルパー(実ポインタの代わりにInteractionコンポーネントを直接遷移させる。
// Headless環境ではPointerイベントが存在しないため、本番の `handle_country_button_click` /
// `handle_start_button` が読む `Interaction` コンポーネントへ直接クリック相当の値を注入する。
// これにより本番システム自体は一切変更せず、本番の入力ハンドラを実際に実行させる。
//
// 注意: `bevy_ui::focus::ui_focus_system` は、カメラのRenderTargetが `Window` でない場合
// (本テストでは `RenderTarget::Image`)、カーソル座標を解決できないため毎フレーム全UI Nodeの
// `Interaction` を強制的に `None` へリセットする(PreUpdate, `UiSystems::Focus`)。そのため
// `world_mut()` からの直接注入をUpdateスケジュール実行前に行っても、同じフレームの
// PreUpdateで即座に上書きされてしまう。これを避けるため、注入専用システムを
// `PreUpdate.after(UiSystems::Focus)` に登録し、ui_focus_systemのリセットの「後」に
// 意図した `Interaction::Pressed` を書き込む。これにより本番の `Update` 側ハンドラ
// (`handle_country_button_click` / `handle_start_button`)が同一フレーム内で確実に
// `Changed<Interaction> == Pressed` を観測できる。)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Resource, Default, Clone, Copy)]
enum PendingClick {
    #[default]
    None,
    CountryButtonIndex(usize),
    StartGameButton,
}

struct InputInjectionPlugin;

impl Plugin for InputInjectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingClick>()
            .add_systems(PreUpdate, apply_pending_click.after(UiSystems::Focus));
    }
}

fn apply_pending_click(
    mut pending: ResMut<PendingClick>,
    mut country_buttons: Query<(Entity, &CountrySelectButton, &mut Interaction)>,
    mut start_button: Query<
        &mut Interaction,
        (With<StartGameButton>, Without<CountrySelectButton>),
    >,
) {
    let action = std::mem::take(&mut *pending);
    match action {
        PendingClick::None => {}
        PendingClick::CountryButtonIndex(index) => {
            let mut entities: Vec<Entity> = country_buttons.iter().map(|(e, _, _)| e).collect();
            entities.sort_by_key(|e| e.index());
            let target = entities.get(index).copied().unwrap_or_else(|| {
                panic!(
                    "[P20-007] Expected at least {} CountrySelectButton entities, found {}",
                    index + 1,
                    entities.len()
                )
            });
            let (_, _, mut interaction) = country_buttons
                .get_mut(target)
                .expect("[P20-007] failed to fetch CountrySelectButton Interaction for injection");
            *interaction = Interaction::Pressed;
        }
        PendingClick::StartGameButton => {
            let mut interaction = start_button
                .single_mut()
                .expect("[P20-007] Expected exactly 1 production StartGameButton entity");
            *interaction = Interaction::Pressed;
        }
    }
}

fn queue_country_button_click(app: &mut App, index: usize) {
    app.world_mut()
        .insert_resource(PendingClick::CountryButtonIndex(index));
}

fn queue_start_game_click(app: &mut App) {
    app.world_mut()
        .insert_resource(PendingClick::StartGameButton);
}

// ─────────────────────────────────────────────────────────────────────────
// 本体テスト
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn ui_headless_render_produces_real_pixels() {
    let mut app = build_headless_app();
    initialize_app(&mut app);

    // ── Adapter/Backend確認: 利用不可な場合はここでpanicしてFAILさせる(スキップしない) ──
    {
        let world = app.world();
        let render_device = world.get_resource::<RenderDevice>();
        assert!(
            render_device.is_some(),
            "[P20-007][FAIL:adapter] RenderDevice resource is not present after App::finish(). \
             WGPU Adapter/Backendの初期化に失敗しています。"
        );
        let adapter_info = world.get_resource::<RenderAdapterInfo>();
        assert!(
            adapter_info.is_some(),
            "[P20-007][FAIL:adapter] RenderAdapterInfo resource is not present after App::finish()."
        );
    }

    let adapter_name;
    let adapter_backend;
    let adapter_device_type;
    let adapter_driver;
    {
        let info = &app.world().resource::<RenderAdapterInfo>().0;
        adapter_name = info.name.clone();
        adapter_backend = format!("{:?}", info.backend);
        adapter_device_type = format!("{:?}", info.device_type);
        adapter_driver = info.driver.clone();
    }
    println!(
        "[P20-007] Adapter: name={adapter_name:?} backend={adapter_backend} device_type={adapter_device_type} driver={adapter_driver:?}"
    );

    // ── フォントAssetがロード済みであることを確認 (default_font feature) ──
    {
        let fonts = app.world().resource::<Assets<Font>>();
        assert!(
            !fonts.is_empty(),
            "[P20-007][FAIL:asset] Assets<Font> is empty; no font asset loaded before UI render."
        );
        let default_font = fonts.get(AssetId::<Font>::default());
        assert!(
            default_font.is_some(),
            "[P20-007][FAIL:asset] default embedded font (AssetId::default()) not present in Assets<Font>."
        );
        println!("[P20-007] Assets<Font> loaded: count={}", fonts.len());
    }

    // ── ウォームアップ: Startup→CountrySelection自動遷移、UI生成、レイアウト、初回描画・readback安定化 ──
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }

    // ── UI Camera / offscreen RenderTarget有効性確認 ──
    {
        let world = app.world_mut();
        let mut cam_q = world
            .query_filtered::<(&Camera, &RenderTarget, Has<IsDefaultUiCamera>), With<GameCamera>>();
        let (camera, render_target, is_default_ui_camera) = cam_q
            .single(world)
            .expect("[P20-007][FAIL:camera] GameCamera entity not found after warmup");
        assert!(
            camera.is_active,
            "[P20-007][FAIL:camera] GameCamera.is_active must be true"
        );
        assert!(
            is_default_ui_camera,
            "[P20-007][FAIL:camera] GameCamera must carry IsDefaultUiCamera so production UI renders onto the offscreen target"
        );
        let capture_target = world.resource::<CaptureTarget>();
        match render_target {
            RenderTarget::Image(img) => {
                assert_eq!(
                    img.handle.id(),
                    capture_target.image.id(),
                    "[P20-007][FAIL:camera] GameCamera RenderTarget::Image does not match the offscreen CaptureTarget image"
                );
            }
            other => panic!(
                "[P20-007][FAIL:camera] GameCamera RenderTarget is not Image, got: {other:?}"
            ),
        }
    }

    // ── 本番UIルートEntity(CountrySelectionRoot)が生成されていることを確認 ──
    {
        let world = app.world_mut();
        let mut root_q = world.query::<(&CountrySelectionRoot, &ComputedNode)>();
        let (_, computed) = root_q.single(world).expect(
            "[P20-007][FAIL:ui-root] production CountrySelectionRoot entity not found after warmup",
        );
        assert!(
            computed.size.x > 0.0 && computed.size.y > 0.0,
            "[P20-007][FAIL:layout] CountrySelectionRoot ComputedNode size is zero ({:?}); UI layout did not complete",
            computed.size
        );
        println!(
            "[P20-007] CountrySelectionRoot ComputedNode size = {:?}",
            computed.size
        );
    }

    let evidence_dir = evidence_dir();

    // ── Checkpoint 1: 国選択画面(デフォルトプレビュー) ──
    let (frame_default, w, h) = capture_and_verify_frame(&app, "country_selection_default");
    assert_eq!(w, CAPTURE_WIDTH);
    assert_eq!(h, CAPTURE_HEIGHT);
    let analysis_default = analyze_frame(&frame_default, w, h);
    println!(
        "[P20-007] checkpoint=country_selection_default non_bg={} unique_colors={} bbox={:?} bg_ref={:?}",
        analysis_default.non_background_count,
        analysis_default.unique_color_count,
        analysis_default.bbox,
        analysis_default.background_ref
    );
    assert!(
        analysis_default.unique_color_count > 1,
        "[P20-007][FAIL:pixels] country_selection_default: output is a single flat color, not real UI content"
    );
    assert!(
        analysis_default.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-007][FAIL:pixels] country_selection_default: non-background pixel count {} below minimum {}",
        analysis_default.non_background_count,
        MIN_NON_BACKGROUND_PIXELS
    );
    if let Some((min_x, min_y, max_x, max_y)) = analysis_default.bbox {
        assert!(
            max_x < w && max_y < h,
            "[P20-007][FAIL:pixels] bbox out of image bounds"
        );
        println!(
            "[P20-007] checkpoint=country_selection_default bbox=({min_x},{min_y})-({max_x},{max_y})"
        );
    }
    save_png(
        &evidence_dir.join("01_country_selection_default.png"),
        &frame_default,
        w,
        h,
    );

    // ── Checkpoint 2: 2番目の国を選択(既知のUI値変更 → PreviewDetailTextが変わる) ──
    queue_country_button_click(&mut app, 1);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    let (frame_preview_changed, _, _) =
        capture_and_verify_frame(&app, "country_selection_after_click");
    let analysis_changed = analyze_frame(&frame_preview_changed, w, h);
    println!(
        "[P20-007] checkpoint=country_selection_after_click non_bg={} unique_colors={} bbox={:?}",
        analysis_changed.non_background_count,
        analysis_changed.unique_color_count,
        analysis_changed.bbox
    );
    let diff_after_click = diff_pixel_count(&frame_default, &frame_preview_changed);
    println!(
        "[P20-007] diff(country_selection_default, country_selection_after_click) = {diff_after_click} pixels"
    );
    assert!(
        diff_after_click >= MIN_DIFF_PIXELS,
        "[P20-007][FAIL:differential] selecting a different country changed only {diff_after_click} pixels (< {MIN_DIFF_PIXELS}); UI state change is not reflected in rendered output"
    );
    save_png(
        &evidence_dir.join("02_country_selection_after_click.png"),
        &frame_preview_changed,
        w,
        h,
    );

    // ── Checkpoint 3: Start Gameを押してPlaying状態へ遷移し、本番TopBarRootを観測 ──
    queue_start_game_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }

    {
        let world = app.world_mut();
        let state = world.resource::<State<GameState>>();
        assert_eq!(
            *state.get(),
            GameState::Playing,
            "[P20-007][FAIL:state] GameState did not transition to Playing after simulated Start Game click"
        );
        let mut root_q = world.query::<(&TopBarRoot, &ComputedNode)>();
        let (_, computed) = root_q
            .single(world)
            .expect("[P20-007][FAIL:ui-root] production TopBarRoot entity not found after transitioning to Playing");
        assert!(
            computed.size.x > 0.0 && computed.size.y > 0.0,
            "[P20-007][FAIL:layout] TopBarRoot ComputedNode size is zero ({:?})",
            computed.size
        );
        println!(
            "[P20-007] TopBarRoot ComputedNode size = {:?}",
            computed.size
        );
    }

    let (frame_playing, _, _) = capture_and_verify_frame(&app, "playing_topbar");
    let analysis_playing = analyze_frame(&frame_playing, w, h);
    println!(
        "[P20-007] checkpoint=playing_topbar non_bg={} unique_colors={} bbox={:?}",
        analysis_playing.non_background_count,
        analysis_playing.unique_color_count,
        analysis_playing.bbox
    );
    assert!(
        analysis_playing.unique_color_count > 1,
        "[P20-007][FAIL:pixels] playing_topbar: output is a single flat color"
    );
    assert!(
        analysis_playing.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-007][FAIL:pixels] playing_topbar: non-background pixel count {} below minimum {}",
        analysis_playing.non_background_count,
        MIN_NON_BACKGROUND_PIXELS
    );
    let diff_playing_vs_previous = diff_pixel_count(&frame_preview_changed, &frame_playing);
    println!(
        "[P20-007] diff(country_selection_after_click, playing_topbar) = {diff_playing_vs_previous} pixels"
    );
    assert!(
        diff_playing_vs_previous >= MIN_DIFF_PIXELS,
        "[P20-007][FAIL:differential] transitioning to Playing (TopBarRoot) changed only {diff_playing_vs_previous} pixels"
    );
    save_png(
        &evidence_dir.join("03_playing_topbar.png"),
        &frame_playing,
        w,
        h,
    );

    println!("[P20-007] All headless UI render assertions passed.");
    println!(
        "[P20-007] SUMMARY adapter_name={adapter_name:?} backend={adapter_backend} resolution={w}x{h} format=Rgba8UnormSrgb \
         non_bg(default={}, after_click={}, playing={}) diff(default->after_click={diff_after_click}, after_click->playing={diff_playing_vs_previous})",
        analysis_default.non_background_count,
        analysis_changed.non_background_count,
        analysis_playing.non_background_count,
    );
}
