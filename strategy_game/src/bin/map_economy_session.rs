//! P32-020 (S1): 新マップ経済セッションの起動用バイナリ(**検証用**。正式な世界設定・ゲーム開始ではない)。
//!
//! 起動(カレントは `strategy_game/`):
//!   `cargo run --bin map_economy_session`                       既定シナリオ m4e(資源は m4d のもの)、操作国家は未選択
//!   `cargo run --bin map_economy_session -- --scenario m4d`     シナリオを明示(m4d は全国家AIで操作できる国家がない)
//!   `cargo run --bin map_economy_session -- --actor 1`          操作国家を明示(手動国家だけ。AI国家・存在しない国家は起動失敗)
//!   `cargo run --bin map_economy_session -- --shots <dir>`      同じコード経路を自動で順に実行して画面を保存して終了する(実マウス操作の代替ではない)
//!
//! 操作: ホイール=ズーム / ドラッグ=パン / 左クリック=選択 / 1,2,3,4=表示モード / G=境界線 / R=河川 / B=ステート境界 /
//!       F=検証地域 / Home=全体表示 / Esc=終了(Tab は無効: 保存機能が無い間にセッションを失わないため)。
//!       上部の国家ボタンで操作国家を選び、「+1日」「+7日」「+30日」で日付を進める(停止状態が基本。自動では進まない)。
//!
//! **登録するプラグインは `DefaultPlugins`・`HexPreviewPlugin`(+ワールド設定)・`EconomySessionPlugin` だけ。**
//! 旧ゲームの `AppPlugin`(`GameTimePlugin`)・`EconomyPlugin`・`SaveGamePlugin` 等は登録しないので、旧経路の日次・月次 System は
//! 起動しない。通常起動・終了でファイルは作らない(保存は S3 まで実装しない)。

use bevy::log::{DEFAULT_FILTER, LogPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use strategy_game::building::data::BuildingType;
use strategy_game::economy::map_state::ConstructionOrderId;
use strategy_game::economy::map_session::{SCREEN_DEFAULT_SCENARIO, SCENARIOS, SessionPaths, scenario_by_id};
use strategy_game::economy::map_economy_file::SaveLocation;
use strategy_game::hexmap::save_view::{SaveLocationRes, SaveUi};
use strategy_game::hexmap::economy_view::{EconomySessionPlugin, SessionCommand, SessionConfig};
use strategy_game::hexmap::render::HexViewMode;
use strategy_game::hexmap::view::{HexPreviewPlugin, HexSelection, HexViewSettings, LoadedHexMap};
use strategy_game::hexmap::world_view::{LoadedWorld, WorldPreviewPaths};
use strategy_game::world::{CellStatus, MapStateId, cell_status};

#[derive(Resource)]
struct ShotDir(std::path::PathBuf);
#[derive(Resource)]
struct ShotSaveBase(std::path::PathBuf);

struct Args {
    scenario: String,
    actor: Option<u32>,
    shots: Option<std::path::PathBuf>,
    /// 保存先の基準フォルダ(実際の保存先は `<base>/map_economy/`)。既定は `saves`。検証用に一時フォルダを指定できる。
    save_base: Option<std::path::PathBuf>,
}

fn usage_error(msg: &str) -> ! {
    eprintln!("map_economy_session: {msg}");
    eprintln!("usage: map_economy_session [--scenario <id>] [--actor <国家ラベル>] [--shots <dir>] [--save-base <dir>]");
    std::process::exit(2);
}

/// 引数を解析する。未知の引数・不正な値は黙って無視せず、別の設定へフォールバックせずに終了する。
fn parse_args() -> Args {
    let mut args = Args { scenario: SCREEN_DEFAULT_SCENARIO.to_string(), actor: None, shots: None, save_base: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().unwrap_or_else(|| usage_error(&format!("{name} needs a value")));
        match a.as_str() {
            "--scenario" => args.scenario = value("--scenario"),
            "--actor" => {
                let v = value("--actor");
                args.actor = Some(v.parse::<u32>().unwrap_or_else(|_| usage_error(&format!("--actor: \"{v}\" is not a country label"))));
            }
            "--save-base" => args.save_base = Some(std::path::PathBuf::from(value("--save-base"))),
            "--shots" => args.shots = Some(std::path::PathBuf::from(value("--shots"))),
            other => usage_error(&format!("unknown argument \"{other}\"")),
        }
    }
    if scenario_by_id(&args.scenario).is_none() {
        usage_error(&format!(
            "unknown scenario \"{}\" (known: {})",
            args.scenario,
            SCENARIOS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")
        ));
    }
    args
}

fn main() {
    let args = parse_args();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(LogPlugin { filter: format!("{DEFAULT_FILTER}icu_provider=error,"), ..default() })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "新マップ 経済検証セッション(検証用)".to_string(),
                    resolution: (1600u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_systems(PreStartup, strategy_game::localization::install_japanese_capable_font)
    .add_plugins(HexPreviewPlugin { open_at_start: true, world_preview: Some(WorldPreviewPaths::default_preview()) })
    .add_plugins(EconomySessionPlugin {
        config: SessionConfig { scenario_id: args.scenario.clone(), actor: args.actor, paths: SessionPaths::default() },
    });
    if let Some(base) = &args.save_base {
        app.insert_resource(SaveLocationRes(SaveLocation::new(base.clone())));
    }
    if let Some(dir) = args.shots {
        std::fs::create_dir_all(&dir).expect("create shot dir");
        // 自動スクリプトは保存先を明示した一時フォルダだけに書く(既定の saves/ へは書かない)。
        let Some(base) = args.save_base.clone() else { usage_error("--shots requires --save-base <temp dir> (the script writes saves)") };
        app.insert_resource(ShotDir(dir)).insert_resource(ShotSaveBase(base)).add_systems(Update, shot_script);
    }
    app.run();
}

/// 自動スクリプト。実マウス操作の代替ではなく、画面と同じコマンド経路(`SessionCommand`)・選択(`HexSelection`)を順に実行する。
#[allow(clippy::too_many_arguments)]
fn shot_script(
    mut commands: Commands,
    dir: Res<ShotDir>,
    mut frame: Local<u32>,
    selection: Option<ResMut<HexSelection>>,
    settings: Option<ResMut<HexViewSettings>>,
    map: Option<Res<LoadedHexMap>>,
    world: Option<Res<LoadedWorld>>,
    mut out: MessageWriter<SessionCommand>,
    mut exit: MessageWriter<AppExit>,
    save_ui: Option<ResMut<SaveUi>>,
    save_base: Res<ShotSaveBase>,
) {
    *frame += 1;
    let f = *frame;
    let shot = |commands: &mut Commands, name: &str| {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(dir.0.join(name)));
    };
    if f == 60 {
        shot(&mut commands, "01_initial_no_actor.png");
    }
    let (Some(mut selection), Some(mut settings), Some(map), Some(world)) = (selection, settings, map, world) else {
        if f == 90 {
            exit.write(AppExit::Success);
        }
        return;
    };
    let m = &world.data.model;
    let state_cell = |id: u32| {
        let p = m.provinces_of(MapStateId(id))?.first().copied()?;
        let c = m.province(p)?.cell;
        map.0.cell_index(c.q, c.r)
    };
    let pick = |want: &dyn Fn(CellStatus) -> bool| {
        map.0
            .cells()
            .iter()
            .filter(|c| (c.q - 83).abs() <= 8 && (c.r - 81).abs() <= 8)
            .find(|c| want(cell_status(m, &map.0, c.q, c.r)))
            .and_then(|c| map.0.cell_index(c.q, c.r))
    };
    match f {
        65 => selection.0 = state_cell(20),
        85 => shot(&mut commands, "02_state20_selected_no_actor.png"),
        90 => {
            out.write(SessionCommand::AdvanceDays(1));
        }
        105 => shot(&mut commands, "03_advance_rejected_no_actor.png"),
        110 => {
            out.write(SessionCommand::SetActor(Some(3)));
        }
        125 => shot(&mut commands, "04_ai_actor_rejected.png"),
        130 => {
            out.write(SessionCommand::SetActor(Some(1)));
        }
        150 => shot(&mut commands, "05_actor1_state20.png"),
        155 => {
            out.write(SessionCommand::AdvanceDays(1));
        }
        175 => shot(&mut commands, "06_after_1_day.png"),
        180 => {
            out.write(SessionCommand::AdvanceDays(30));
        }
        200 => shot(&mut commands, "07_after_31_days_monthly.png"),
        205 => selection.0 = state_cell(21),
        225 => shot(&mut commands, "08_other_country_state.png"),
        230 => selection.0 = pick(&|s| matches!(s, CellStatus::Water)),
        250 => shot(&mut commands, "09_water.png"),
        255 => selection.0 = pick(&|s| matches!(s, CellStatus::Unassigned { .. })),
        275 => shot(&mut commands, "10_unassigned.png"),
        280 => selection.0 = pick(&|s| matches!(s, CellStatus::OutOfScope)),
        300 => shot(&mut commands, "11_out_of_scope.png"),
        305 => {
            selection.0 = state_cell(22);
            settings.mode = HexViewMode::State;
        }
        325 => shot(&mut commands, "12_state_mode_state22.png"),
        // ---- S2: 建設候補・発注・キュー・進捗・取消し(画面と同じコマンド経路。実マウス操作の代替ではない)
        350 => selection.0 = state_cell(20),
        370 => shot(&mut commands, "13_build_candidates.png"),
        375 => {
            for b in [BuildingType::Farm, BuildingType::Mine, BuildingType::Factory, BuildingType::LoggingCamp, BuildingType::MilitaryFactory, BuildingType::Railway] {
                out.write(SessionCommand::Build { actor: 1, state: MapStateId(20), building: b });
            }
        }
        400 => shot(&mut commands, "14_queue_6_orders.png"),
        405 => {
            out.write(SessionCommand::AdvanceDays(7));
        }
        425 => shot(&mut commands, "15_progress_7_days.png"),
        430 => {
            // 2番目に発注した Mine(注文識別子 国家1の2番)を、識別子を指定して取り消す。
            out.write(SessionCommand::Cancel { actor: 1, order: ConstructionOrderId { country: 1, seq: 2 } });
        }
        450 => shot(&mut commands, "16_after_cancel.png"),
        455 => selection.0 = state_cell(21),
        475 => shot(&mut commands, "17_other_country_state_view_only.png"),
        480 => selection.0 = pick(&|s| matches!(s, CellStatus::Water)),
        500 => shot(&mut commands, "18_water_no_build.png"),
        505 => {
            out.write(SessionCommand::ToggleBuildPanel);
        }
        525 => shot(&mut commands, "19_build_panel_hidden.png"),
        530 => {
            out.write(SessionCommand::ToggleBuildPanel);
        }
        // ---- S3: 保存・一覧・事前確認・復元(画面と同じコマンド経路。実マウス・実キーボード操作の代替ではない)。
        //      保存先は --save-base の一時フォルダだけ。
        550 => {
            if let Some(mut u) = save_ui {
                u.name = "shot_a".to_string();
                u.focus = true;
            }
            out.write(SessionCommand::SaveAs { actor: Some(1), name: "shot_a".to_string(), replace: false });
        }
        570 => shot(&mut commands, "20_saved_shot_a.png"),
        575 => {
            out.write(SessionCommand::AdvanceDays(7));
            // 壊れたファイルと旧V1形式のファイル(一覧に「破損」「非対応」として出る)。保存先は一時フォルダ。
            let dir = save_base.0.join("map_economy");
            std::fs::write(dir.join("shot_broken.mapeco.ron"), "(kind: \"map_economy_state\", format_version: 2, trunc").expect("write broken");
            std::fs::write(dir.join("shot_v1like.mapeco.ron"), "(version: 1, game_time: (year: 1800))").expect("write v1like");
            for i in 1..=10 {
                out.write(SessionCommand::SaveAs { actor: Some(1), name: format!("list_{i:02}"), replace: false });
            }
            out.write(SessionCommand::RefreshSaves);
        }
        600 => shot(&mut commands, "21_list_long_after_7_days.png"),
        605 => {
            out.write(SessionCommand::InspectSave { name: "shot_a".to_string() });
        }
        625 => shot(&mut commands, "22_preview_shot_a.png"),
        630 => {
            out.write(SessionCommand::LoadSave { actor: Some(1), name: "shot_a".to_string() });
        }
        650 => shot(&mut commands, "23_restored_shot_a.png"),
        655 => {
            out.write(SessionCommand::InspectSave { name: "shot_broken".to_string() });
        }
        675 => shot(&mut commands, "24_preview_broken.png"),
        680 => {
            if let Some(mut u) = save_ui {
                u.name = "bad name/../x".to_string();
            }
        }
        700 => shot(&mut commands, "25_invalid_name.png"),
        705 => {
            if let Some(mut u) = save_ui {
                u.name = "shot_a".to_string();
            }
        }
        725 => shot(&mut commands, "26_replace_target.png"),
        730 => {
            out.write(SessionCommand::SaveAs { actor: Some(1), name: "shot_a".to_string(), replace: false });
        }
        750 => shot(&mut commands, "27_replace_refused_without_explicit.png"),
        760 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
