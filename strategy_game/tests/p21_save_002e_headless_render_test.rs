//! P21-SAVE-002E: セーブ/ロードUI導線のHeadless実描画・実操作検証。
//!
//! `tests/ui_headless_render_test.rs`(P20-007)と同じ手法(本番`main.rs`と同一のプラグイン
//! 構成をWindowなしのoffscreen `RenderTarget::Image`へ接続し、実フレームを実行して
//! GPUからピクセルをreadbackしPNGとして保存)を踏襲する。人間がウィンドウを操作する
//! 代わりに、本番の`Interaction`コンポーネントへ実際のクリック相当の値を注入し、
//! 本番の`handle_save_button`/`handle_load_button`/`handle_load_confirm_button`/
//! `handle_load_cancel_button`/`handle_save_requests`/`handle_load_requests`を
//! 実際に実行させる。§14の「実際のcargo run手動検証」の自動化された代替。
//!
//! 実行方法:
//!   cargo test --test p21_save_002e_headless_render_test -- --nocapture
//!
//! 既存の`verification_logs/p20-007`等、固定済み証跡PNGは一切上書きしない
//! (専用の`verification_logs/phase-21/p21-save-002e/screenshots`へのみ保存する)。
//! セーブファイルの読み書きは一意な一時ディレクトリへ`SaveFileConfig`を上書きして行い、
//! 実リポジトリの`saves/`には一切触れない。

use bevy::app::PluginsState;
use bevy::asset::AssetId;
use bevy::camera::RenderTarget;
use bevy::ecs::system::SystemState;
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
use strategy_game::app::time::{GameDate, GamePaused};
use strategy_game::building::BuildingPlugin;
use strategy_game::country::CountryPlugin;
use strategy_game::debug::DebugPlugin;
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::logistics::LogisticsPlugin;
use strategy_game::map::MapPlugin;
use strategy_game::map::camera::{CameraDragState, GameCamera};
use strategy_game::military::MilitaryPlugin;
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::{
    LastLoadOutcome, LoadExecutionCount, LoadGamePlugin, LoadOutcome, SaveExecutionCount,
    SaveFileConfig, SaveGamePlugin, SavePathConfig,
};
use strategy_game::state::StatePlugin;
use strategy_game::ui::UiPlugin;
use strategy_game::ui::country_selection::{
    CountrySelectButton, CountrySelectionRoot, StartGameButton,
};
use strategy_game::ui::load_confirm::{LoadCancelButton, LoadConfirmButton, LoadConfirmRoot};
use strategy_game::ui::notification::GameNotification;
use strategy_game::ui::top_bar::{LoadButton, SaveButton, TopBarRoot};
use strategy_game::war::WarPlugin;

const CAPTURE_WIDTH: u32 = 640;
const CAPTURE_HEIGHT: u32 = 480;
const BG_TOLERANCE: i16 = 16;
const MIN_NON_BACKGROUND_PIXELS: usize = 300;
const MIN_DIFF_PIXELS: usize = 20;
const WARMUP_FRAMES: usize = 30;
const SETTLE_FRAMES: usize = 20;

// ─────────────────────────────────────────────────────────────────────────
// Headless offscreen readback インフラ(P20-007 `ui_headless_render_test.rs`と同一パターン。
// 別コンパイル単位[統合テストは各ファイルが独立クレート]のため、共有モジュール化はせず
// そのまま複製する。既存2本の踏襲であり、本ラウンドで新規に設計したものではない)。
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
            label: Some("p21_save_002e_headless_readback_buffer"),
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
            .expect("[P21-SAVE-002E] readback texture format must have a known block size");

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
                            .expect("[P21-SAVE-002E] padded_bytes_per_row must be non-zero")
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
                .expect("[P21-SAVE-002E] Failed to send map_async completion"),
            Err(err) => {
                panic!("[P21-SAVE-002E] readback: Failed to map GPU buffer for CPU read: {err}")
            }
        });

        render_device.poll(PollType::wait_indefinitely()).expect(
            "[P21-SAVE-002E] readback: Failed to poll RenderDevice while waiting for map_async",
        );

        r.recv()
            .expect("[P21-SAVE-002E] readback: Failed to receive map_async completion signal");

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
        "[P21-SAVE-002E] production GameCamera entity (map::camera::setup_camera) must exist before headless capture setup runs",
    );
    commands.entity(camera_entity).insert((
        RenderTarget::Image(render_target_handle.into()),
        IsDefaultUiCamera,
    ));
}

fn build_headless_app(save_file_dir: &Path) -> App {
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

    // main.rs と同一順序・同一プラグイン構成(本番のSave/Load UI登録経路そのもの)。
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
        .add_plugins(DebugPlugin)
        .add_plugins(SaveGamePlugin)
        .add_plugins(LoadGamePlugin);

    // 実リポジトリの`saves/`には一切触れない。一意な一時ディレクトリへ差し替える。
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: save_file_dir.join("savegame_v1.ron"),
        },
    });

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
            "[P21-SAVE-002E] Timed out waiting for plugins to finish adding (>60s). Possible Adapter/Backend initialization stall."
        );
    }
    app.finish();
    app.cleanup();
}

// ─────────────────────────────────────────────────────────────────────────
// フレーム解析・PNG保存ヘルパー(P20-007と同一)
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
        "[P21-SAVE-002E] frame buffers must be same size to diff"
    );
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(pa, pb)| pa != pb)
        .count()
}

fn save_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            panic!("[P21-SAVE-002E] Failed to create PNG output dir {parent:?}: {e}")
        });
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)
        .unwrap_or_else(|e| panic!("[P21-SAVE-002E] Failed to save PNG {path:?}: {e}"));
}

fn evidence_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("verification_logs/phase-21/p21-save-002e/screenshots")
}

fn capture_and_verify_frame(app: &App, label: &str) -> (Vec<u8>, u32, u32) {
    let target = app.world().resource::<CaptureTarget>().clone();
    let latest = app.world().resource::<LatestFrame>();
    let raw = latest.0.clone().unwrap_or_else(|| {
        panic!("[P21-SAVE-002E][FAIL:readback] checkpoint '{label}': LatestFrame is None.")
    });

    let rgba = unpad_rgba(&raw, target.width, target.height);
    let expected_len = (target.width * target.height * 4) as usize;
    assert_eq!(
        rgba.len(),
        expected_len,
        "[P21-SAVE-002E][FAIL:readback] checkpoint '{label}': unpadded frame byte length mismatch"
    );

    (rgba, target.width, target.height)
}

// ─────────────────────────────────────────────────────────────────────────
// UI操作ヘルパー: 実クリックの代わりに`Interaction`コンポーネントへ直接注入する
// (P20-007と同一手法。`PreUpdate.after(UiSystems::Focus)`で注入し、本番の
// `Update`側ハンドラが同一フレーム内で`Changed<Interaction> == Pressed`を観測できるようにする)。
// ─────────────────────────────────────────────────────────────────────────

#[derive(Resource, Default, Clone, Copy)]
enum PendingClick {
    #[default]
    None,
    CountryButtonIndex(usize),
    StartGameButton,
    SaveButton,
    LoadButton,
    LoadConfirmButton,
    LoadCancelButton,
}

struct InputInjectionPlugin;

impl Plugin for InputInjectionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingClick>()
            .add_systems(PreUpdate, apply_pending_click.after(UiSystems::Focus));
    }
}

/// 各`With<T>`クエリの`&mut Interaction`アクセスは、静的にはBevyから互いに重複しうると
/// 判定される(実際には各Entityが持つマーカーは互いに排他だが、型システムだけでは
/// 証明できない)ため、`ParamSet`へ束ねて「1フレームに高々1つだけ触る」設計を
/// 実行時に保証する(P20-007はCountrySelectButton/StartGameButtonの2種類だけだった
/// ため`Without<T>`の相互排他フィルタで足りたが、本ラウンドはボタン種別が6種類に
/// 増え、全組み合わせを`Without`で書き下すより`ParamSet`の方が安全)。
#[allow(clippy::type_complexity)]
fn apply_pending_click(
    mut pending: ResMut<PendingClick>,
    mut set: ParamSet<(
        Query<(Entity, &CountrySelectButton, &mut Interaction)>,
        Query<&mut Interaction, With<StartGameButton>>,
        Query<&mut Interaction, With<SaveButton>>,
        Query<&mut Interaction, With<LoadButton>>,
        Query<&mut Interaction, With<LoadConfirmButton>>,
        Query<&mut Interaction, With<LoadCancelButton>>,
    )>,
) {
    let action = std::mem::take(&mut *pending);
    match action {
        PendingClick::None => {}
        PendingClick::CountryButtonIndex(index) => {
            let mut country_buttons = set.p0();
            let mut entities: Vec<Entity> = country_buttons.iter().map(|(e, _, _)| e).collect();
            entities.sort_by_key(|e| e.index());
            let target = entities.get(index).copied().unwrap_or_else(|| {
                panic!(
                    "[P21-SAVE-002E] Expected at least {} CountrySelectButton entities, found {}",
                    index + 1,
                    entities.len()
                )
            });
            let (_, _, mut interaction) = country_buttons
                .get_mut(target)
                .expect("[P21-SAVE-002E] failed to fetch CountrySelectButton Interaction");
            *interaction = Interaction::Pressed;
        }
        PendingClick::StartGameButton => {
            let mut query = set.p1();
            let mut interaction = query
                .single_mut()
                .expect("[P21-SAVE-002E] Expected exactly 1 production StartGameButton entity");
            *interaction = Interaction::Pressed;
        }
        PendingClick::SaveButton => {
            let mut query = set.p2();
            let mut interaction = query
                .single_mut()
                .expect("[P21-SAVE-002E] Expected exactly 1 production SaveButton entity");
            *interaction = Interaction::Pressed;
        }
        PendingClick::LoadButton => {
            let mut query = set.p3();
            let mut interaction = query
                .single_mut()
                .expect("[P21-SAVE-002E] Expected exactly 1 production LoadButton entity");
            *interaction = Interaction::Pressed;
        }
        PendingClick::LoadConfirmButton => {
            let mut query = set.p4();
            let mut interaction = query
                .single_mut()
                .expect("[P21-SAVE-002E] Expected exactly 1 production LoadConfirmButton entity");
            *interaction = Interaction::Pressed;
        }
        PendingClick::LoadCancelButton => {
            let mut query = set.p5();
            let mut interaction = query
                .single_mut()
                .expect("[P21-SAVE-002E] Expected exactly 1 production LoadCancelButton entity");
            *interaction = Interaction::Pressed;
        }
    }
}

fn queue_click(app: &mut App, click: PendingClick) {
    app.world_mut().insert_resource(click);
}

/// `GameNotification`を1個の永続`SystemState`で読み続けるカーソル。フレームごとに
/// 新しい`SystemState`を作ると読み取りカーソルがリセットされ、Bevyのdouble-buffer上に
/// まだ残っている過去のメッセージを再度読んでしまう(`save::runtime`のテストで
/// 実際に踏んだ既知の罠)。
struct NotificationCursor(SystemState<MessageReader<'static, 'static, GameNotification>>);

impl NotificationCursor {
    fn new(app: &mut App) -> Self {
        Self(SystemState::new(app.world_mut()))
    }

    fn drain(&mut self, app: &mut App) -> Vec<String> {
        self.0
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .map(|n| n.message.clone())
            .collect()
    }
}

fn set_camera_transform(app: &mut App, translation: Vec3, scale: Vec3) {
    let mut query = app
        .world_mut()
        .query_filtered::<&mut Transform, With<GameCamera>>();
    let mut transform = query
        .single_mut(app.world_mut())
        .expect("[P21-SAVE-002E] Expected exactly 1 GameCamera entity");
    transform.translation = translation;
    transform.scale = scale;
}

fn get_camera_transform(app: &mut App) -> Transform {
    let mut query = app
        .world_mut()
        .query_filtered::<&Transform, With<GameCamera>>();
    *query
        .single(app.world())
        .expect("[P21-SAVE-002E] Expected exactly 1 GameCamera entity")
}

// ─────────────────────────────────────────────────────────────────────────
// 本体テスト
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn save_load_ui_headless_render_produces_real_pixels_and_real_state_changes() {
    let save_dir = std::env::temp_dir().join(format!(
        "strategy_game_p21_save_002e_headless_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct CleanupGuard(PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = CleanupGuard(save_dir.clone());

    let mut app = build_headless_app(&save_dir);
    initialize_app(&mut app);

    {
        let world = app.world();
        assert!(
            world.get_resource::<RenderDevice>().is_some(),
            "[P21-SAVE-002E][FAIL:adapter] RenderDevice resource is not present after App::finish()."
        );
        let info = &world.resource::<RenderAdapterInfo>().0;
        println!(
            "[P21-SAVE-002E] Adapter: name={:?} backend={:?} device_type={:?}",
            info.name, info.backend, info.device_type
        );
    }

    {
        let fonts = app.world().resource::<Assets<Font>>();
        assert!(
            !fonts.is_empty(),
            "[P21-SAVE-002E][FAIL:asset] Assets<Font> is empty."
        );
        assert!(
            fonts.get(AssetId::<Font>::default()).is_some(),
            "[P21-SAVE-002E][FAIL:asset] default embedded font not present."
        );
    }

    let mut notif_cursor = NotificationCursor::new(&mut app);

    // ── ウォームアップ: CountrySelection UI生成・レイアウト・初回描画安定化 ──
    for _ in 0..WARMUP_FRAMES {
        app.update();
    }
    notif_cursor.drain(&mut app); // warmup中に紛れ込んだメッセージがあれば捨てる(通常は無い)

    {
        let world = app.world_mut();
        let mut root_q = world.query::<(&CountrySelectionRoot, &ComputedNode)>();
        let (_, computed) = root_q
            .single(world)
            .expect("[P21-SAVE-002E][FAIL:ui-root] CountrySelectionRoot not found");
        assert!(computed.size.x > 0.0 && computed.size.y > 0.0);
    }

    // ── Kingdom of Arcadia(CountryId(0))を選択してゲーム開始 ──
    queue_click(&mut app, PendingClick::CountryButtonIndex(0));
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    queue_click(&mut app, PendingClick::StartGameButton);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }

    {
        let state = app.world().resource::<State<GameState>>();
        assert_eq!(*state.get(), GameState::Playing);
    }

    // ── UI Camera / offscreen RenderTarget有効性確認(本番GameCameraが実際に
    // このoffscreenバッファへ描画していることを保証する。P20-007と同じ確認) ──
    {
        let world = app.world_mut();
        let mut cam_q = world
            .query_filtered::<(&Camera, &RenderTarget, Has<IsDefaultUiCamera>), With<GameCamera>>();
        let (camera, render_target, is_default_ui_camera) = cam_q
            .single(world)
            .expect("[P21-SAVE-002E][FAIL:camera] GameCamera entity not found after warmup");
        assert!(camera.is_active);
        assert!(is_default_ui_camera);
        let capture_target = world.resource::<CaptureTarget>();
        match render_target {
            RenderTarget::Image(img) => {
                assert_eq!(
                    img.handle.id(),
                    capture_target.image.id(),
                    "[P21-SAVE-002E][FAIL:camera] GameCamera RenderTarget::Image does not match the offscreen CaptureTarget image"
                );
            }
            other => panic!(
                "[P21-SAVE-002E][FAIL:camera] GameCamera RenderTarget is not Image, got: {other:?}"
            ),
        }
    }

    let evidence_dir = evidence_dir();

    // ── Checkpoint 1: Playing突入直後、TopBarにSave/Loadボタンが実在する ──
    {
        let world = app.world_mut();
        let mut root_q = world.query::<(&TopBarRoot, &ComputedNode)>();
        let (_, computed) = root_q
            .single(world)
            .expect("[P21-SAVE-002E][FAIL:ui-root] TopBarRoot not found");
        assert!(computed.size.x > 0.0 && computed.size.y > 0.0);

        let save_count = world.query::<&SaveButton>().iter(world).count();
        let load_count = world.query::<&LoadButton>().iter(world).count();
        assert_eq!(
            save_count, 1,
            "exactly 1 SaveButton must exist in the real running game"
        );
        assert_eq!(
            load_count, 1,
            "exactly 1 LoadButton must exist in the real running game"
        );
    }
    let camera_at_start = get_camera_transform(&mut app);
    assert_eq!(
        camera_at_start,
        Transform::IDENTITY,
        "GameCamera must start at the canonical default Transform"
    );

    let (frame_topbar, w, h) = capture_and_verify_frame(&app, "topbar_with_save_load_buttons");
    let analysis_topbar = analyze_frame(&frame_topbar, w, h);
    assert!(analysis_topbar.unique_color_count > 1);
    assert!(analysis_topbar.non_background_count >= MIN_NON_BACKGROUND_PIXELS);
    save_png(
        &evidence_dir.join("01_playing_topbar_with_save_load_buttons.png"),
        &frame_topbar,
        w,
        h,
    );

    // ── 状態A記録(セーブ対象になる、現在の実ゲーム状態) ──
    let state_a_year = app.world().resource::<GameDate>().year;
    assert!(
        !save_dir.join("savegame_v1.ron").exists(),
        "starting the game and reaching Playing must not create a save file before any Save click"
    );

    // ── Checkpoint 2: Saveボタンを実クリック。ファイル生成・通知1件・実行回数1回を確認 ──
    // `GameNotification`はBevyのdouble-buffer上に約2フレームしか残らないため、書き込まれた
    // 直後(クリックを注入したその1フレーム: PreUpdateで注入→Updateで`SaveRequestMessage`
    // 発行→PostUpdateで`handle_save_requests`が実際に書き込む)にすぐ`drain`する。
    // その後に見た目安定化のための残りのsettleフレームを回す(先にsettleし切ってから
    // drainすると、20フレーム分の間にバッファから消えてしまい0件観測になる実際の罠)。
    queue_click(&mut app, PendingClick::SaveButton);
    app.update();
    let save_notifications = notif_cursor.drain(&mut app);
    for _ in 0..(SETTLE_FRAMES - 1) {
        app.update();
    }

    assert!(
        save_dir.join("savegame_v1.ron").exists(),
        "clicking Save must create the save file on disk"
    );
    assert_eq!(app.world().resource::<SaveExecutionCount>().0, 1);
    assert_eq!(
        save_notifications.len(),
        1,
        "clicking Save must emit exactly one notification, got {save_notifications:?}"
    );
    assert!(!save_notifications[0].is_empty());

    let (frame_after_save, _, _) = capture_and_verify_frame(&app, "after_save_click");
    save_png(
        &evidence_dir.join("02_after_save_click.png"),
        &frame_after_save,
        w,
        h,
    );

    // ── 状態Bへ変更(セーブ後に大きくプレイを進めた想定) + カメラを動かす ──
    {
        let mut date = app.world_mut().resource_mut::<GameDate>();
        date.year += 3;
    }
    set_camera_transform(&mut app, Vec3::new(400.0, -250.0, 0.0), Vec3::splat(2.2));
    for _ in 0..5 {
        app.update();
    }
    let camera_state_b = get_camera_transform(&mut app);
    assert_ne!(
        camera_state_b, camera_at_start,
        "camera must have visibly moved away from default before testing the load-reset"
    );

    // ── Checkpoint 3: Loadボタンを実クリック。即ロードせず確認ダイアログだけが開く ──
    queue_click(&mut app, PendingClick::LoadButton);
    app.update();
    let load_button_notifications = notif_cursor.drain(&mut app);
    assert!(
        load_button_notifications.is_empty(),
        "opening the confirm dialog must not itself emit a notification, got {load_button_notifications:?}"
    );
    for _ in 0..(SETTLE_FRAMES - 1) {
        app.update();
    }

    assert_eq!(
        app.world().resource::<LoadExecutionCount>().0,
        0,
        "the first Load click must not execute a load yet"
    );
    assert_eq!(
        app.world().resource::<GameDate>().year,
        state_a_year + 3,
        "opening the confirm dialog must not change game state"
    );
    {
        let world = app.world_mut();
        let mut query = world.query::<(&LoadConfirmRoot, &Node)>();
        let (_, node) = query
            .single(world)
            .expect("[P21-SAVE-002E][FAIL:ui-root] LoadConfirmRoot not found");
        assert_eq!(
            node.display,
            Display::Flex,
            "the confirm dialog must be visible"
        );
    }

    let (frame_confirm_open, _, _) = capture_and_verify_frame(&app, "load_confirm_dialog_open");
    let diff_confirm_vs_save = diff_pixel_count(&frame_after_save, &frame_confirm_open);
    assert!(
        diff_confirm_vs_save >= MIN_DIFF_PIXELS,
        "opening the load confirm dialog must visibly change the rendered frame ({diff_confirm_vs_save} px changed)"
    );
    save_png(
        &evidence_dir.join("03_load_confirm_dialog_open.png"),
        &frame_confirm_open,
        w,
        h,
    );

    // ── Checkpoint 4: キャンセル。何も変わらずダイアログだけが閉じる ──
    queue_click(&mut app, PendingClick::LoadCancelButton);
    app.update();
    let cancel_notifications = notif_cursor.drain(&mut app);
    for _ in 0..(SETTLE_FRAMES - 1) {
        app.update();
    }

    assert_eq!(
        app.world().resource::<LoadExecutionCount>().0,
        0,
        "Cancel must never load"
    );
    assert_eq!(
        app.world().resource::<GameDate>().year,
        state_a_year + 3,
        "Cancel must not change game state"
    );
    assert_eq!(
        get_camera_transform(&mut app),
        camera_state_b,
        "Cancel must not touch the camera"
    );
    assert!(
        cancel_notifications.is_empty(),
        "Cancel must not emit any notification, got {cancel_notifications:?}"
    );
    {
        let world = app.world_mut();
        let mut query = world.query::<(&LoadConfirmRoot, &Node)>();
        let (_, node) = query.single(world).unwrap();
        assert_eq!(node.display, Display::None, "Cancel must close the dialog");
    }

    let (frame_after_cancel, _, _) = capture_and_verify_frame(&app, "after_cancel_dialog_closed");
    let diff_cancel_vs_open = diff_pixel_count(&frame_confirm_open, &frame_after_cancel);
    assert!(
        diff_cancel_vs_open >= MIN_DIFF_PIXELS,
        "closing the dialog via Cancel must visibly change the rendered frame"
    );
    save_png(
        &evidence_dir.join("04_after_cancel_dialog_closed.png"),
        &frame_after_cancel,
        w,
        h,
    );

    // ── Checkpoint 5: Load → 確認ダイアログの「ロード」を実クリック。実際にロードする ──
    queue_click(&mut app, PendingClick::LoadButton);
    for _ in 0..SETTLE_FRAMES {
        app.update();
    }
    queue_click(&mut app, PendingClick::LoadConfirmButton);
    app.update();
    let load_notifications = notif_cursor.drain(&mut app);
    for _ in 0..(SETTLE_FRAMES - 1) {
        app.update();
    }

    assert_eq!(
        app.world().resource::<LoadExecutionCount>().0,
        1,
        "confirming Load must execute exactly once"
    );
    match &app.world().resource::<LastLoadOutcome>().0 {
        Some(LoadOutcome::Success { .. }) => {}
        other => panic!("[P21-SAVE-002E] expected a successful load, got {other:?}"),
    }
    assert_eq!(
        app.world().resource::<GameDate>().year,
        state_a_year,
        "load must revert the date to state A"
    );
    assert!(
        app.world().resource::<GamePaused>().0,
        "a successful load must pause the game"
    );
    let camera_after_load = get_camera_transform(&mut app);
    assert_eq!(
        camera_after_load,
        Transform::IDENTITY,
        "a successful load must visually reset the camera to the canonical default, not to the saved position (camera is never part of save data)"
    );
    assert_eq!(app.world().resource::<CameraDragState>().drag_start, None);

    assert_eq!(
        load_notifications.len(),
        1,
        "confirming Load must emit exactly one notification, got {load_notifications:?}"
    );
    assert!(!load_notifications[0].is_empty());

    {
        let world = app.world_mut();
        let mut query = world.query::<(&LoadConfirmRoot, &Node)>();
        let (_, node) = query.single(world).unwrap();
        assert_eq!(
            node.display,
            Display::None,
            "the confirm dialog must not remain open after a successful load"
        );
    }

    let (frame_after_load, _, _) = capture_and_verify_frame(&app, "after_load_success");
    let analysis_after_load = analyze_frame(&frame_after_load, w, h);
    assert!(analysis_after_load.unique_color_count > 1);
    let diff_load_vs_cancel = diff_pixel_count(&frame_after_cancel, &frame_after_load);
    assert!(
        diff_load_vs_cancel >= MIN_DIFF_PIXELS,
        "a successful load (camera reset + date/paused UI change) must visibly change the rendered frame"
    );
    save_png(
        &evidence_dir.join("05_after_load_success.png"),
        &frame_after_load,
        w,
        h,
    );

    // ── ロード後に再度セーブできること(ロード直後にセーブ経路が壊れていないこと) ──
    queue_click(&mut app, PendingClick::SaveButton);
    app.update();
    let resave_notifications = notif_cursor.drain(&mut app);
    for _ in 0..(SETTLE_FRAMES - 1) {
        app.update();
    }
    assert_eq!(
        app.world().resource::<SaveExecutionCount>().0,
        2,
        "re-saving after a load must succeed"
    );
    assert_eq!(resave_notifications.len(), 1);

    println!(
        "[P21-SAVE-002E] SUMMARY resolution={w}x{h} diffs(save->confirm_open={diff_confirm_vs_save}, \
         open->cancel={diff_cancel_vs_open}, cancel->load_success={diff_load_vs_cancel}) \
         non_bg(topbar={}, after_load={})",
        analysis_topbar.non_background_count, analysis_after_load.non_background_count,
    );
    println!("[P21-SAVE-002E] All headless save/load UI render assertions passed.");
}
