//! P20-009: Headless実GPU描画によるja-JP/en-US/切替後の本番UI検証。
//!
//! P20-007 (`tests/ui_headless_render_test.rs`)の Headless実GPU・offscreen描画・
//! PNG readback方式をそのまま再利用する(本番`UiPlugin`・本番`GameCamera`・
//! 実際のRenderGraph実行・実ピクセルreadback。偽UIへの置き換えは行わない)。
//!
//! 実行方法:
//!   cargo test --test p20_009_localization_headless_render_test -- --nocapture
//!
//! 検証内容 (P20-009要件):
//! - 本番UiPluginとGameCameraを使用し、Font Assetが正常ロードされる
//! - ja-JP / en-US / 切替後en-US / 再度ja-JP のそれぞれで非背景ピクセルが存在する
//! - 日本語と英語で描画結果に差がある(ピクセル差分)
//! - 言語切り替え後もUIが消失しない(非背景ピクセル数が閾値以上)
//! - PNGを証拠として保存する
//! - ピクセル差分だけでなく、翻訳キー・Text内容のアサートも併用する
//! - 言語切り替え前後でシミュレーション状態(国庫・人口・戦争数)が不変
//! - `LocalizedText`を持つ全Textに欠落キーマーカーが一切出現しない
//! - P20-007の判定条件(閾値・アサート)は弱めない

use bevy::app::PluginsState;
use bevy::asset::AssetId;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode, PollType,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat, TextureUsages,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue};
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
use strategy_game::country::country_ai::CountryAiRegistry;
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::debug::DebugPlugin;
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::localization::{
    CurrentLocale, LanguageToggleButton, Locale, LocalizedText, MISSING_KEY_MARKER_PREFIX,
};
use strategy_game::logistics::LogisticsPlugin;
use strategy_game::map::MapPlugin;
use strategy_game::map::camera::GameCamera;
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::UiPlugin;
use strategy_game::ui::country_selection::{
    CountrySelectButton, CountrySelectionRoot, StartGameButton,
};
use strategy_game::ui::top_bar::{TopBarPlayerInfoText, TopBarRoot};
use strategy_game::war::WarPlugin;
use strategy_game::war::data::WarRegistry;

const CAPTURE_WIDTH: u32 = 640;
const CAPTURE_HEIGHT: u32 = 480;
const BG_TOLERANCE: i16 = 16;
const MIN_NON_BACKGROUND_PIXELS: usize = 300;
const MIN_DIFF_PIXELS: usize = 50;
const WARMUP_FRAMES: usize = 30;
const SETTLE_FRAMES: usize = 20;

// ─────────────────────────────────────────────────────────────────────────
// Headless offscreen readback インフラ (P20-007と同型)
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
            label: Some("p20_009_headless_ui_readback_buffer"),
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
            .expect("[P20-009] readback texture format must have a known block size");

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
                            .expect("[P20-009] padded_bytes_per_row must be non-zero")
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
                .expect("[P20-009] Failed to send map_async completion"),
            Err(err) => panic!("[P20-009] readback: Failed to map GPU buffer for CPU read: {err}"),
        });

        render_device
            .poll(PollType::wait_indefinitely())
            .expect("[P20-009] readback: Failed to poll RenderDevice while waiting for map_async");

        r.recv()
            .expect("[P20-009] readback: Failed to receive map_async completion signal");

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
        "[P20-009] production GameCamera entity must exist before headless capture setup runs",
    );
    commands.entity(camera_entity).insert((
        RenderTarget::Image(render_target_handle.into()),
        IsDefaultUiCamera,
    ));
}

// ─────────────────────────────────────────────────────────────────────────
// App構築: main.rsと同一のプラグイン構成(P20-009はUiPlugin経由で自動的に
// LocalizationPluginを含む。main.rs自体は変更不要)。
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
            .disable::<WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );

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

fn initialize_app(app: &mut App) {
    let start = std::time::Instant::now();
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
        assert!(
            start.elapsed().as_secs() < 60,
            "[P20-009] Timed out waiting for plugins to finish adding (>60s)."
        );
    }
    app.finish();
    app.cleanup();
}

// ─────────────────────────────────────────────────────────────────────────
// フレーム解析・PNG保存ヘルパー (P20-007と同型)
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
}

fn analyze_frame(rgba: &[u8], width: u32, height: u32) -> FrameAnalysis {
    let background_ref = [rgba[0], rgba[1], rgba[2], rgba[3]];
    let mut non_background_count = 0usize;
    let mut colors: HashSet<[u8; 4]> = HashSet::new();

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
            }
        }
    }

    FrameAnalysis {
        non_background_count,
        unique_color_count: colors.len(),
    }
}

fn diff_pixel_count(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(
        a.len(),
        b.len(),
        "[P20-009] frame buffers must be same size to diff"
    );
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(pa, pb)| pa != pb)
        .count()
}

fn save_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            panic!("[P20-009] Failed to create PNG output dir {parent:?}: {e}")
        });
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)
        .unwrap_or_else(|e| panic!("[P20-009] Failed to save PNG {path:?}: {e}"));
}

fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("verification_logs/p20-009/screenshots")
}

fn capture_and_verify_frame(app: &App, label: &str) -> (Vec<u8>, u32, u32) {
    let target = app.world().resource::<CaptureTarget>().clone();
    let latest = app.world().resource::<LatestFrame>();
    let raw = latest.0.clone().unwrap_or_else(|| {
        panic!("[P20-009][FAIL:readback] checkpoint '{label}': LatestFrame is None.")
    });

    let rgba = unpad_rgba(&raw, target.width, target.height);
    let expected_len = (target.width * target.height * 4) as usize;
    assert_eq!(
        rgba.len(),
        expected_len,
        "[P20-009][FAIL:readback] checkpoint '{label}': unpadded frame byte length mismatch"
    );

    (rgba, target.width, target.height)
}

// ─────────────────────────────────────────────────────────────────────────
// UI操作ヘルパー (P20-007と同型の PreUpdate.after(UiSystems::Focus) 注入方式)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Resource, Default, Clone, Copy)]
enum PendingClick {
    #[default]
    None,
    CountryButtonIndex(usize),
    StartGameButton,
    LanguageToggleButton,
}

struct InputInjectionPlugin;

impl Plugin for InputInjectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingClick>()
            .add_systems(PreUpdate, apply_pending_click.after(UiSystems::Focus));
    }
}

#[allow(clippy::type_complexity)]
fn apply_pending_click(
    mut pending: ResMut<PendingClick>,
    mut country_buttons: Query<(Entity, &CountrySelectButton, &mut Interaction)>,
    mut start_button: Query<
        &mut Interaction,
        (With<StartGameButton>, Without<CountrySelectButton>),
    >,
    mut language_button: Query<
        &mut Interaction,
        (
            With<LanguageToggleButton>,
            Without<CountrySelectButton>,
            Without<StartGameButton>,
        ),
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
                    "[P20-009] Expected at least {} CountrySelectButton entities, found {}",
                    index + 1,
                    entities.len()
                )
            });
            let (_, _, mut interaction) = country_buttons
                .get_mut(target)
                .expect("[P20-009] failed to fetch CountrySelectButton Interaction for injection");
            *interaction = Interaction::Pressed;
        }
        PendingClick::StartGameButton => {
            let mut interaction = start_button
                .single_mut()
                .expect("[P20-009] Expected exactly 1 production StartGameButton entity");
            *interaction = Interaction::Pressed;
        }
        PendingClick::LanguageToggleButton => {
            let mut interaction = language_button
                .single_mut()
                .expect("[P20-009] Expected exactly 1 production LanguageToggleButton entity");
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

fn queue_language_toggle_click(app: &mut App) {
    app.world_mut()
        .insert_resource(PendingClick::LanguageToggleButton);
}

// ─────────────────────────────────────────────────────────────────────────
// シミュレーション状態・欠落キー検証ヘルパー
// ─────────────────────────────────────────────────────────────────────────

struct SimSnapshot {
    treasury: f64,
    available_manpower: u64,
    total_population: u64,
    war_count: usize,
    ai_state_count: usize,
}

fn snapshot_sim_state(app: &mut App) -> SimSnapshot {
    let world = app.world_mut();
    let player_country = world.resource::<PlayerCountry>();
    let country_registry = world.resource::<CountryRegistry>();
    let state_registry = world.resource::<StateRegistry>();
    let war_registry = world.resource::<WarRegistry>();
    let country_ai_registry = world.resource::<CountryAiRegistry>();

    let country = player_country
        .0
        .and_then(|id| country_registry.get(id))
        .expect("[P20-009] player country must exist after Start Game");

    SimSnapshot {
        treasury: country.treasury,
        available_manpower: country.available_manpower,
        total_population: state_registry.states.iter().map(|s| s.population).sum(),
        war_count: war_registry.wars.len(),
        ai_state_count: country_ai_registry.ai_states.len(),
    }
}

fn assert_snapshots_equal(before: &SimSnapshot, after: &SimSnapshot, label: &str) {
    assert_eq!(
        before.treasury, after.treasury,
        "[P20-009][{label}] treasury changed"
    );
    assert_eq!(
        before.available_manpower, after.available_manpower,
        "[P20-009][{label}] available_manpower changed"
    );
    assert_eq!(
        before.total_population, after.total_population,
        "[P20-009][{label}] total_population changed"
    );
    assert_eq!(
        before.war_count, after.war_count,
        "[P20-009][{label}] war_count changed"
    );
    assert_eq!(
        before.ai_state_count, after.ai_state_count,
        "[P20-009][{label}] ai_state_count changed"
    );
}

fn find_missing_markers(app: &mut App) -> Vec<String> {
    let world = app.world_mut();
    let mut query = world.query::<(&Text, &LocalizedText)>();
    query
        .iter(world)
        .filter(|(text, _)| text.0.contains(MISSING_KEY_MARKER_PREFIX))
        .map(|(text, marker)| format!("key='{}' text='{}'", marker.key, text.0))
        .collect()
}

fn top_bar_text(app: &mut App) -> String {
    let world = app.world_mut();
    let mut query = world.query::<(&Text, &TopBarPlayerInfoText)>();
    let (text, _) = query
        .iter(world)
        .next()
        .expect("[P20-009] TopBarPlayerInfoText must exist while Playing");
    text.0.clone()
}

fn current_locale(app: &App) -> Locale {
    app.world().resource::<CurrentLocale>().0
}

/// 1フレームだけキー入力を「押下」させる(本番の`just_pressed`ハンドラを実際に発火させる)。
fn tap_key(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(key);
}

// ─────────────────────────────────────────────────────────────────────────
// 本体テスト
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn localization_headless_render_ja_en_switch_and_back() {
    let mut app = build_headless_app();
    initialize_app(&mut app);

    {
        let world = app.world();
        assert!(
            world.get_resource::<RenderDevice>().is_some(),
            "[P20-009][FAIL:adapter] no RenderDevice"
        );
        let fonts = world.resource::<Assets<Font>>();
        assert!(
            !fonts.is_empty(),
            "[P20-009][FAIL:asset] Assets<Font> is empty"
        );
        assert!(
            fonts.get(AssetId::<Font>::default()).is_some(),
            "[P20-009][FAIL:asset] default font (Noto Sans JP) not present at AssetId::default()"
        );
    }

    for _ in 0..WARMUP_FRAMES {
        app.update();
    }

    assert_eq!(
        current_locale(&app),
        Locale::JaJp,
        "[P20-009] default locale must be ja-JP"
    );

    {
        let world = app.world_mut();
        let mut root_q = world.query::<(&CountrySelectionRoot, &ComputedNode)>();
        let (_, computed) = root_q
            .single(world)
            .expect("[P20-009][FAIL:ui-root] CountrySelectionRoot not found after warmup");
        assert!(computed.size.x > 0.0 && computed.size.y > 0.0);

        let mut cam_q =
            world.query_filtered::<(&RenderTarget, Has<IsDefaultUiCamera>), With<GameCamera>>();
        let (render_target, is_default_ui_camera) = cam_q
            .single(world)
            .expect("[P20-009][FAIL:camera] GameCamera entity not found after warmup");
        assert!(
            is_default_ui_camera,
            "[P20-009][FAIL:camera] GameCamera must carry IsDefaultUiCamera"
        );
        let capture_target = world.resource::<CaptureTarget>();
        match render_target {
            RenderTarget::Image(img) => {
                assert_eq!(
                    img.handle.id(),
                    capture_target.image.id(),
                    "[P20-009][FAIL:camera] GameCamera RenderTarget::Image does not match the offscreen CaptureTarget image"
                );
            }
            other => panic!(
                "[P20-009][FAIL:camera] GameCamera RenderTarget is not Image, got: {other:?}"
            ),
        }
    }

    let evidence_dir = evidence_dir();
    let mut png_hashes: Vec<(String, String)> = Vec::new();

    // ── Checkpoint 1: 国選択画面, ja-JP (既定言語) ──
    let (frame_ja1, w, h) = capture_and_verify_frame(&app, "country_selection_ja_jp");
    assert_eq!(w, CAPTURE_WIDTH);
    assert_eq!(h, CAPTURE_HEIGHT);
    let a1 = analyze_frame(&frame_ja1, w, h);
    println!(
        "[P20-009] checkpoint=country_selection_ja_jp non_bg={} unique_colors={}",
        a1.non_background_count, a1.unique_color_count
    );
    assert!(
        a1.unique_color_count > 1,
        "[P20-009][FAIL:pixels] country_selection_ja_jp: single flat color"
    );
    assert!(
        a1.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-009][FAIL:pixels] country_selection_ja_jp: non-bg pixels {} below {}",
        a1.non_background_count,
        MIN_NON_BACKGROUND_PIXELS
    );
    save_png(
        &evidence_dir.join("01_country_selection_ja_jp.png"),
        &frame_ja1,
        w,
        h,
    );
    assert!(
        find_missing_markers(&mut app).is_empty(),
        "[P20-009] missing keys at checkpoint 1 (ja-JP country selection): {:?}",
        find_missing_markers(&mut app)
    );

    // ── Checkpoint 2: 言語切替ボタンをクリック -> en-US ──
    queue_language_toggle_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert_eq!(
        current_locale(&app),
        Locale::EnUs,
        "[P20-009] language toggle click must switch to en-US"
    );

    let (frame_en1, _, _) = capture_and_verify_frame(&app, "country_selection_en_us");
    let a2 = analyze_frame(&frame_en1, w, h);
    println!(
        "[P20-009] checkpoint=country_selection_en_us non_bg={} unique_colors={}",
        a2.non_background_count, a2.unique_color_count
    );
    assert!(
        a2.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-009][FAIL:pixels] UI vanished after switching to en-US"
    );
    let diff_ja1_en1 = diff_pixel_count(&frame_ja1, &frame_en1);
    println!("[P20-009] diff(ja_jp, en_us) country_selection = {diff_ja1_en1} pixels");
    assert!(
        diff_ja1_en1 >= MIN_DIFF_PIXELS,
        "[P20-009][FAIL:differential] switching ja-JP -> en-US changed only {diff_ja1_en1} pixels; rendering does not reflect language change"
    );
    save_png(
        &evidence_dir.join("02_country_selection_en_us.png"),
        &frame_en1,
        w,
        h,
    );
    assert!(
        find_missing_markers(&mut app).is_empty(),
        "[P20-009] missing keys at checkpoint 2 (en-US country selection): {:?}",
        find_missing_markers(&mut app)
    );

    // ── Checkpoint 3: 言語切替ボタンを再度クリック -> ja-JPへ戻す ──
    queue_language_toggle_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert_eq!(
        current_locale(&app),
        Locale::JaJp,
        "[P20-009] second toggle click must switch back to ja-JP"
    );

    let (frame_ja2, _, _) = capture_and_verify_frame(&app, "country_selection_ja_jp_again");
    let a3 = analyze_frame(&frame_ja2, w, h);
    println!(
        "[P20-009] checkpoint=country_selection_ja_jp_again non_bg={} unique_colors={}",
        a3.non_background_count, a3.unique_color_count
    );
    assert!(
        a3.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-009][FAIL:pixels] UI vanished after switching back to ja-JP"
    );
    let diff_en1_ja2 = diff_pixel_count(&frame_en1, &frame_ja2);
    println!("[P20-009] diff(en_us, ja_jp_again) country_selection = {diff_en1_ja2} pixels");
    assert!(
        diff_en1_ja2 >= MIN_DIFF_PIXELS,
        "[P20-009][FAIL:differential] switching en-US -> ja-JP changed only {diff_en1_ja2} pixels"
    );
    // 同一状態への再描画は決定的なため、最初のja-JP描画とほぼ一致するはず(僅かなdiffは許容)。
    let diff_ja1_ja2 = diff_pixel_count(&frame_ja1, &frame_ja2);
    println!(
        "[P20-009] diff(ja_jp, ja_jp_again) round-trip = {diff_ja1_ja2} pixels (expected small)"
    );
    save_png(
        &evidence_dir.join("03_country_selection_ja_jp_again.png"),
        &frame_ja2,
        w,
        h,
    );
    assert!(
        find_missing_markers(&mut app).is_empty(),
        "[P20-009] missing keys at checkpoint 3 (ja-JP again, country selection): {:?}",
        find_missing_markers(&mut app)
    );

    // ── Playingへ遷移(P20-007と同一の本番ハンドラ経路) ──
    queue_country_button_click(&mut app, 1);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    queue_start_game_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert_eq!(
        *app.world().resource::<State<GameState>>().get(),
        GameState::Playing,
        "[P20-009][FAIL:state] did not transition to Playing"
    );
    {
        let world = app.world_mut();
        let mut root_q = world.query::<(&TopBarRoot, &ComputedNode)>();
        let (_, computed) = root_q
            .single(world)
            .expect("[P20-009][FAIL:ui-root] TopBarRoot not found after transitioning to Playing");
        assert!(computed.size.x > 0.0 && computed.size.y > 0.0);
    }

    // 折りたたみパネルを全て開き、それぞれのUI更新Systemを最低1回実行させておく
    // (未開放のままLocalizedText::default()が残らないことを保証し、後続の言語
    // 切り替えテストが全パネルのコンテンツを実際に検証できるようにする)。
    tap_key(&mut app, KeyCode::KeyR); // Research
    tap_key(&mut app, KeyCode::KeyP); // Politics
    tap_key(&mut app, KeyCode::KeyN); // Peace (旧: Politicsと同じKeyPを共有していたが、
    // 重複解消のためKeyNへ変更)
    tap_key(&mut app, KeyCode::KeyG); // Diplomacy (旧: WASDカメラ移動のKeyDと衝突していたため
    // KeyGへ変更)
    tap_key(&mut app, KeyCode::KeyM); // Military
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert!(
        find_missing_markers(&mut app).is_empty(),
        "[P20-009] missing keys after opening all panels (ja-JP): {:?}",
        find_missing_markers(&mut app)
    );

    let snapshot_before = snapshot_sim_state(&mut app);
    let ja_playing_text = top_bar_text(&mut app);
    assert!(!ja_playing_text.is_empty());

    let (frame_playing_ja, _, _) = capture_and_verify_frame(&app, "playing_ja_jp");
    let a4 = analyze_frame(&frame_playing_ja, w, h);
    println!(
        "[P20-009] checkpoint=playing_ja_jp non_bg={} unique_colors={}",
        a4.non_background_count, a4.unique_color_count
    );
    assert!(a4.non_background_count >= MIN_NON_BACKGROUND_PIXELS);
    save_png(
        &evidence_dir.join("04_playing_ja_jp.png"),
        &frame_playing_ja,
        w,
        h,
    );
    assert!(find_missing_markers(&mut app).is_empty());

    // ── Playing中に言語切替(TopBar側のボタン) -> en-US ──
    queue_language_toggle_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert_eq!(current_locale(&app), Locale::EnUs);

    let snapshot_after_en = snapshot_sim_state(&mut app);
    assert_snapshots_equal(&snapshot_before, &snapshot_after_en, "playing ja->en");

    let en_playing_text = top_bar_text(&mut app);
    assert_ne!(
        ja_playing_text, en_playing_text,
        "[P20-009] TopBar text must change after switching to en-US"
    );
    let treasury_digits = format!("{:.0}", snapshot_before.treasury);
    assert!(
        en_playing_text.contains(&treasury_digits) && ja_playing_text.contains(&treasury_digits),
        "[P20-009] treasury figure '{treasury_digits}' must survive the language switch: ja='{ja_playing_text}' en='{en_playing_text}'"
    );

    let (frame_playing_en, _, _) = capture_and_verify_frame(&app, "playing_en_us");
    let a5 = analyze_frame(&frame_playing_en, w, h);
    println!(
        "[P20-009] checkpoint=playing_en_us non_bg={} unique_colors={}",
        a5.non_background_count, a5.unique_color_count
    );
    assert!(
        a5.non_background_count >= MIN_NON_BACKGROUND_PIXELS,
        "[P20-009][FAIL:pixels] UI vanished (Playing, en-US)"
    );
    let diff_playing_ja_en = diff_pixel_count(&frame_playing_ja, &frame_playing_en);
    println!("[P20-009] diff(playing_ja_jp, playing_en_us) = {diff_playing_ja_en} pixels");
    assert!(
        diff_playing_ja_en >= MIN_DIFF_PIXELS,
        "[P20-009][FAIL:differential] Playing screen language switch changed only {diff_playing_ja_en} pixels"
    );
    save_png(
        &evidence_dir.join("05_playing_en_us.png"),
        &frame_playing_en,
        w,
        h,
    );
    assert!(find_missing_markers(&mut app).is_empty());

    // ── Playing中に再度ja-JPへ戻す ──
    queue_language_toggle_click(&mut app);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    assert_eq!(current_locale(&app), Locale::JaJp);

    let snapshot_after_ja_again = snapshot_sim_state(&mut app);
    assert_snapshots_equal(&snapshot_before, &snapshot_after_ja_again, "playing en->ja");

    let ja_playing_text_again = top_bar_text(&mut app);
    assert_eq!(
        ja_playing_text, ja_playing_text_again,
        "[P20-009] switching back to ja-JP during Playing must reproduce the original text exactly"
    );

    let (frame_playing_ja2, _, _) = capture_and_verify_frame(&app, "playing_ja_jp_again");
    let a6 = analyze_frame(&frame_playing_ja2, w, h);
    println!(
        "[P20-009] checkpoint=playing_ja_jp_again non_bg={} unique_colors={}",
        a6.non_background_count, a6.unique_color_count
    );
    assert!(a6.non_background_count >= MIN_NON_BACKGROUND_PIXELS);
    let diff_playing_en_ja2 = diff_pixel_count(&frame_playing_en, &frame_playing_ja2);
    println!("[P20-009] diff(playing_en_us, playing_ja_jp_again) = {diff_playing_en_ja2} pixels");
    assert!(diff_playing_en_ja2 >= MIN_DIFF_PIXELS);
    save_png(
        &evidence_dir.join("06_playing_ja_jp_again.png"),
        &frame_playing_ja2,
        w,
        h,
    );
    assert!(
        find_missing_markers(&mut app).is_empty(),
        "[P20-009] missing keys found at final checkpoint: {:?}",
        find_missing_markers(&mut app)
    );

    for (name, path) in [
        (
            "01_country_selection_ja_jp.png",
            evidence_dir.join("01_country_selection_ja_jp.png"),
        ),
        (
            "02_country_selection_en_us.png",
            evidence_dir.join("02_country_selection_en_us.png"),
        ),
        (
            "03_country_selection_ja_jp_again.png",
            evidence_dir.join("03_country_selection_ja_jp_again.png"),
        ),
        (
            "04_playing_ja_jp.png",
            evidence_dir.join("04_playing_ja_jp.png"),
        ),
        (
            "05_playing_en_us.png",
            evidence_dir.join("05_playing_en_us.png"),
        ),
        (
            "06_playing_ja_jp_again.png",
            evidence_dir.join("06_playing_ja_jp_again.png"),
        ),
    ] {
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("[P20-009] failed to re-read {path:?}: {e}"));
        use std::fmt::Write as _;
        let mut hash_hex = String::new();
        for byte in simple_sha256(&bytes) {
            let _ = write!(hash_hex, "{byte:02x}");
        }
        png_hashes.push((name.to_string(), hash_hex));
    }
    for (name, hash) in &png_hashes {
        println!("[P20-009] PNG SHA-256 {name} = {hash}");
    }

    println!("[P20-009] All localization headless render assertions passed.");
    println!(
        "[P20-009] SUMMARY treasury={} diff(ja1,en1)={diff_ja1_en1} diff(en1,ja2)={diff_en1_ja2} \
         diff(playing_ja,playing_en)={diff_playing_ja_en} diff(playing_en,playing_ja2)={diff_playing_en_ja2}",
        snapshot_before.treasury
    );
}

/// 依存クレートを追加せずSHA-256を計算する最小実装(証拠ログ用途のみ)。
fn simple_sha256(data: &[u8]) -> [u8; 32] {
    // P20-009の検証ログはSHA-256を要求するが、新規crateを追加しない方針のため、
    // FIPS 180-4に基づく最小実装をテスト内に直接記述する。
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
