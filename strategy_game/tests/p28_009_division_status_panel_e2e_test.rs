//! P28-009: 選択Divisionの戦力・装備・組織率・補給・戦闘準備率の軍事パネル表示の統合テスト。
//!
//! 実データ(6か国28州)+実`MilitaryPanelPlugin`+実日次System(日次補給配分)を通して、
//! 次の3点を固定する。
//!
//! - 複数選択時に、Divisionごとの値が正しいIDへ結び付く(補給0のDivisionを隠さない)。
//! - 日次更新・状態変化・消滅・save→load後の値がパネルへ反映される。
//! - パネルを開閉しても、補給配分・戦闘に使う値が変わらない。
//!
//! 表示は正規データ(`MilitaryRegistry::division_supply_fulfillment`と
//! `calculate_combat_readiness_with_supply`)のみを使う。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::{DayChangedMessage, GameDate, MonthChangedMessage};
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionId, StateId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::localization::MISSING_KEY_MARKER_PREFIX;
use strategy_game::map::division_selection::SelectedDivision;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{
    LastLoadOutcome, LoadOutcome, LoadRequestMessage, SaveFileConfig,
};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::state::data::StateRegistry;
use strategy_game::ui::division_status_view::percent_from_fraction;
use strategy_game::ui::military_panel::{
    MilitaryPanelPlugin, MilitaryPanelState, MilitaryPanelText,
};
use strategy_game::war::WarPlugin;

const PLAYER: CountryId = CountryId(0);

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p28_009_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app(dir: &std::path::Path) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(PopulationPlugin);
    app.add_plugins(MilitaryPanelPlugin);
    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .insert_resource(strategy_game::map::frontline_render::FrontlineRenderSettings::default())
        .insert_resource(strategy_game::map::frontline_selection::FrontlineSelectMode::default())
        .insert_resource(
            strategy_game::map::offensive_line_selection::OffensiveLineEditMode::default(),
        )
        .add_message::<strategy_game::map::frontline_selection::FrontlineAssignmentAttempted>()
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();
    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);
    app.insert_resource(SaveFileConfig {
        path: SavePathConfig {
            final_path: dir.join("savegame_v1.ron"),
        },
    });
    app.update();
    app.insert_resource(PlayerCountry(Some(PLAYER)));
    app.insert_state(GameState::Playing);
    app.update();
    app
}

fn open_panel(app: &mut App) {
    app.world_mut().resource_mut::<MilitaryPanelState>().open = true;
    app.update();
}

fn panel_text(app: &mut App) -> String {
    app.world_mut()
        .query_filtered::<&Text, With<MilitaryPanelText>>()
        .single(app.world())
        .expect("MilitaryPanelText must be spawned")
        .0
        .clone()
}

fn select(app: &mut App, ids: &[DivisionId]) {
    app.world_mut()
        .resource_mut::<SelectedDivision>()
        .select_only_many(ids.iter().copied());
}

fn state_name(app: &App, id: StateId) -> String {
    app.world()
        .resource::<StateRegistry>()
        .get(id)
        .unwrap()
        .name
        .clone()
}

#[allow(clippy::too_many_arguments)]
fn add_division(
    app: &mut App,
    state: StateId,
    status: DivisionStatus,
    manpower: u64,
    equipment: f64,
    max_equipment: f64,
    organization: f32,
    supply_entry: Option<f64>,
) -> DivisionId {
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    let real_def_id = mr
        .divisions
        .values()
        .next()
        .expect("実マップは初期Divisionを持つ")
        .def_id;
    let id = mr.add_division(Division {
        id: DivisionId(0),
        owner: PLAYER,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: 10_000,
        equipment,
        max_equipment,
        organization,
        max_organization: 50.0,
        morale: 50.0,
        max_morale: 50.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status,
        def_id: real_def_id,
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    });
    if let Some(v) = supply_entry {
        mr.division_supply_fulfillment.insert(id, v);
    }
    id
}

/// `#<id> @`で始まるヘッダー行と、その直後の詳細行(複数選択表示の1Divisionぶん)を返す。
fn block_for(text: &str, id: DivisionId) -> (String, String) {
    let lines: Vec<&str> = text.lines().collect();
    let prefix = format!("  #{} @", id.0);
    let i = lines
        .iter()
        .position(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no block for {id:?} in:\n{text}"));
    (lines[i].to_string(), lines[i + 1].to_string())
}

fn advance_one_day(app: &mut App) {
    let (old_month, next) = {
        let d = app.world().resource::<GameDate>();
        (d.month, d.add_days(1))
    };
    {
        let mut d = app.world_mut().resource_mut::<GameDate>();
        d.year = next.year;
        d.month = next.month;
        d.day = next.day;
    }
    let mut s: SystemState<(
        MessageWriter<DayChangedMessage>,
        MessageWriter<MonthChangedMessage>,
    )> = SystemState::new(app.world_mut());
    {
        let (mut dw, mut mw) = s.get_mut(app.world_mut()).unwrap();
        dw.write(DayChangedMessage {
            year: next.year,
            month: next.month,
            day: next.day,
        });
        if next.month != old_month {
            mw.write(MonthChangedMessage {
                year: next.year,
                month: next.month,
            });
        }
    }
    s.apply(app.world_mut());
    app.update();
}

fn set_supplies(app: &mut App, c: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(c)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitarySupplies, value);
}

// ─────────────────────────────────────────────────────────────────────────

#[test]
fn multi_selection_binds_each_divisions_own_supply_to_its_own_id_in_either_order() {
    for supply_on_lower_id in [0.0, 1.0] {
        let dir = TempDir(unique_temp_dir("multi"));
        let mut app = setup_app(&dir.0);
        let (s1, s2) = (StateId(0), StateId(1));
        // 低いIDと高いIDで補給・装備・組織率・状態・州をすべて別々にする。
        let low = add_division(
            &mut app,
            s1,
            DivisionStatus::Fighting,
            10_000,
            100.0,
            100.0,
            0.0,
            Some(supply_on_lower_id),
        );
        let high = add_division(
            &mut app,
            s2,
            DivisionStatus::Retreating,
            7_600,
            25.0,
            100.0,
            50.0,
            Some(1.0 - supply_on_lower_id),
        );
        assert!(low.0 < high.0);
        select(&mut app, &[high, low]);
        open_panel(&mut app);
        let text = panel_text(&mut app);
        assert!(!text.contains(MISSING_KEY_MARKER_PREFIX), "{text}");

        let (low_head, low_detail) = block_for(&text, low);
        let (high_head, high_detail) = block_for(&text, high);
        // ID・州・状態の対応
        assert!(
            low_head.contains(&state_name(&app, s1)) && low_head.contains("戦闘中"),
            "{low_head}"
        );
        assert!(
            high_head.contains(&state_name(&app, s2)) && high_head.contains("退却中"),
            "{high_head}"
        );
        // 値の対応(低いID: 戦力100%/装備100%/組織率0%、高いID: 戦力76%/装備25%/組織率100%)
        let low_supply = if supply_on_lower_id == 0.0 {
            "0%"
        } else {
            "100%"
        };
        let high_supply = if supply_on_lower_id == 0.0 {
            "100%"
        } else {
            "0%"
        };
        assert!(
            low_detail.contains(&format!("補給 {low_supply} ")),
            "{low_detail}"
        );
        assert!(low_detail.contains("装備 100/100 (100%)"), "{low_detail}");
        assert!(low_detail.contains("組織率 0%"), "{low_detail}");
        assert!(
            high_detail.contains(&format!("補給 {high_supply} ")),
            "{high_detail}"
        );
        assert!(
            high_detail.contains("戦力 7600/10000 (76.0%)"),
            "{high_detail}"
        );
        assert!(high_detail.contains("装備 25/100 (25.0%)"), "{high_detail}");
        assert!(high_detail.contains("組織率 100%"), "{high_detail}");
        // 補給0のDivisionは、補給が制約として明示される
        let zero_detail = if supply_on_lower_id == 0.0 {
            &low_detail
        } else {
            &high_detail
        };
        assert!(zero_detail.contains("準備率 0% [補給]"), "{zero_detail}");
    }
}

#[test]
fn single_selection_shows_supply_equipment_organization_and_readiness_with_clear_labels() {
    let dir = TempDir(unique_temp_dir("single"));
    let mut app = setup_app(&dir.0);
    let id = add_division(
        &mut app,
        StateId(0),
        DivisionStatus::Idle,
        10_000,
        100.0,
        100.0,
        50.0,
        Some(0.0),
    );
    select(&mut app, &[id]);
    open_panel(&mut app);
    let text = panel_text(&mut app);

    assert!(
        text.contains("戦力(manpower): 10000 / 10000 (100%)"),
        "{text}"
    );
    assert!(
        text.contains("装備(MilitaryEquipment): 100 / 100 (充足率 100%)"),
        "{text}"
    );
    assert!(text.contains("組織率: 50 / 50 (100%)"), "{text}");
    // 補給は「直近の日次補給判定」でそのDivisionへ配分された値であり、国家備蓄日数ではない
    let supply_line = text
        .lines()
        .find(|l| l.starts_with("補給(直近の日次補給判定): "))
        .unwrap_or_else(|| panic!("no supply line in:\n{text}"));
    assert!(supply_line.contains("0%"), "{supply_line}");
    assert!(
        supply_line.contains("国家のMilitarySupplies備蓄日数ではありません"),
        "{supply_line}"
    );
    // 戦闘準備率は制約項目を示し、最終戦闘力とは言わない
    let readiness = text
        .lines()
        .find(|l| l.starts_with("戦闘準備率: "))
        .unwrap();
    assert!(
        readiness.contains("0%") && readiness.contains("制約: 補給"),
        "{readiness}"
    );
    assert!(
        readiness.contains("組織率・地形補正は含みません"),
        "{readiness}"
    );
}

#[test]
fn display_updates_when_the_daily_value_changes_and_distinguishes_uncomputed_and_vanished() {
    let dir = TempDir(unique_temp_dir("update"));
    let mut app = setup_app(&dir.0);
    let id = add_division(
        &mut app,
        StateId(0),
        DivisionStatus::Idle,
        10_000,
        100.0,
        100.0,
        50.0,
        Some(1.0),
    );
    select(&mut app, &[id]);
    open_panel(&mut app);
    assert!(panel_text(&mut app).contains("補給(直近の日次補給判定): 100%"));

    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .insert(id, 0.0);
    app.update();
    assert!(panel_text(&mut app).contains("補給(直近の日次補給判定): 0%"));

    // 装備・組織率・戦力の変化も反映される
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        let d = mr.divisions.get_mut(&id).unwrap();
        d.equipment = 40.0;
        d.organization = 10.0;
        d.manpower = 5_000;
    }
    app.update();
    let text = panel_text(&mut app);
    assert!(
        text.contains("戦力(manpower): 5000 / 10000 (50.0%)"),
        "{text}"
    );
    assert!(
        text.contains("装備(MilitaryEquipment): 40 / 100 (充足率 40.0%)"),
        "{text}"
    );
    assert!(text.contains("組織率: 10 / 50 (20.0%)"), "{text}");

    // 未計算: 0%と断定せず、戦闘計算では満額扱いであることを示す
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .remove(&id);
    app.update();
    let text = panel_text(&mut app);
    assert!(text.contains("補給(直近の日次補給判定): 未計算"), "{text}");
    assert!(!text.contains("補給(直近の日次補給判定): 0%"), "{text}");

    // 異常値
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .insert(id, 1.5);
    app.update();
    assert!(panel_text(&mut app).contains("補給(直近の日次補給判定): 異常値 (1.5)"));

    // 消滅したら古い値を表示し続けない
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .remove_division(id);
    app.update();
    let text = panel_text(&mut app);
    assert!(text.contains(&format!("#{}: 消滅済み", id.0)), "{text}");
    assert!(!text.contains("補給(直近の日次補給判定)"), "{text}");
}

#[test]
fn required_equipment_zero_is_not_reported_as_shortage_and_does_not_break_the_panel() {
    let dir = TempDir(unique_temp_dir("noequip"));
    let mut app = setup_app(&dir.0);
    let id = add_division(
        &mut app,
        StateId(0),
        DivisionStatus::Idle,
        10_000,
        0.0,
        0.0,
        50.0,
        Some(1.0),
    );
    select(&mut app, &[id]);
    open_panel(&mut app);
    let text = panel_text(&mut app);
    assert!(text.contains("装備(MilitaryEquipment): 必要量0"), "{text}");
    assert!(!text.contains("充足率を算出できません"), "{text}");
    assert!(text.contains("戦闘準備率: 100%"), "{text}");
}

#[test]
fn panel_shows_the_value_the_real_daily_supply_system_allocated() {
    let dir = TempDir(unique_temp_dir("daily"));
    let mut app = setup_app(&dir.0);
    let chosen = {
        let mr = app.world().resource::<MilitaryRegistry>();
        let mut ids: Vec<DivisionId> = mr
            .divisions
            .values()
            .filter(|d| d.owner == PLAYER && d.manpower > 0)
            .map(|d| d.id)
            .collect();
        ids.sort_by_key(|i| i.0);
        *ids.first().expect("実マップのArcadiaは初期Divisionを持つ")
    };
    select(&mut app, &[chosen]);
    open_panel(&mut app);

    // 国家備蓄を枯渇させ、実際の日次補給Systemに配分させる。
    set_supplies(&mut app, PLAYER, 0.0);
    for _ in 0..3 {
        advance_one_day(&mut app);
    }
    let allocated = {
        let mr = app.world().resource::<MilitaryRegistry>();
        mr.division_supply_fulfillment.get(&chosen).copied()
    };
    let allocated = allocated.expect("日次補給配分後はこのDivisionの値が確定している");
    let text = panel_text(&mut app);
    let expected = format!(
        "補給(直近の日次補給判定): {}",
        percent_from_fraction(allocated)
    );
    assert!(
        text.contains(&expected),
        "expected '{expected}' in:\n{text}"
    );
    assert!(allocated < 1.0, "備蓄0なら満額ではない: {allocated}");

    // 備蓄を十分に戻して再配分させると、表示も新しい配分値へ更新される。
    set_supplies(&mut app, PLAYER, 1_000_000.0);
    for _ in 0..3 {
        advance_one_day(&mut app);
    }
    let allocated2 = app
        .world()
        .resource::<MilitaryRegistry>()
        .division_supply_fulfillment
        .get(&chosen)
        .copied()
        .unwrap();
    let text = panel_text(&mut app);
    assert!(
        text.contains(&format!(
            "補給(直近の日次補給判定): {}",
            percent_from_fraction(allocated2)
        )),
        "{text}"
    );
    assert!(
        allocated2 > allocated,
        "備蓄回復後は配分が増える: {allocated} -> {allocated2}"
    );
}

#[test]
fn opening_and_closing_the_panel_does_not_change_supply_or_game_state() {
    let dir = TempDir(unique_temp_dir("noeffect"));
    let mut app = setup_app(&dir.0);
    let id = add_division(
        &mut app,
        StateId(0),
        DivisionStatus::Idle,
        8_000,
        60.0,
        100.0,
        30.0,
        Some(0.4),
    );
    select(&mut app, &[id]);
    let before = {
        let mr = app.world().resource::<MilitaryRegistry>();
        (
            mr.division_supply_fulfillment.get(&id).copied(),
            format!("{:?}", mr.divisions[&id]),
            app.world()
                .resource::<CountryRegistry>()
                .get(PLAYER)
                .unwrap()
                .stockpile
                .get(ResourceType::MilitarySupplies),
        )
    };
    for open in [true, false, true, false, true] {
        app.world_mut().resource_mut::<MilitaryPanelState>().open = open;
        app.update();
    }
    let mr = app.world().resource::<MilitaryRegistry>();
    assert_eq!(mr.division_supply_fulfillment.get(&id).copied(), before.0);
    assert_eq!(format!("{:?}", mr.divisions[&id]), before.1);
    let stock = app
        .world()
        .resource::<CountryRegistry>()
        .get(PLAYER)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitarySupplies);
    assert_eq!(stock, before.2);
}

#[test]
fn display_after_save_load_shows_the_loaded_values() {
    let dir = TempDir(unique_temp_dir("saveload"));
    let mut app = setup_app(&dir.0);
    let id = add_division(
        &mut app,
        StateId(0),
        DivisionStatus::Idle,
        9_000,
        80.0,
        100.0,
        25.0,
        Some(0.35),
    );
    select(&mut app, &[id]);
    open_panel(&mut app);
    let before = panel_text(&mut app);
    assert!(
        before.contains("補給(直近の日次補給判定): 35.0%"),
        "{before}"
    );

    // 保存後に値を変え、ロードで保存時の値に戻ることを確認する。
    {
        let mut s: SystemState<MessageWriter<SaveRequestMessage>> =
            SystemState::new(app.world_mut());
        s.get_mut(app.world_mut())
            .unwrap()
            .write(SaveRequestMessage);
        s.apply(app.world_mut());
        app.update();
    }
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .division_supply_fulfillment
        .insert(id, 1.0);
    app.update();
    assert!(panel_text(&mut app).contains("補給(直近の日次補給判定): 100%"));
    {
        let mut s: SystemState<MessageWriter<LoadRequestMessage>> =
            SystemState::new(app.world_mut());
        s.get_mut(app.world_mut())
            .unwrap()
            .write(LoadRequestMessage);
        s.apply(app.world_mut());
        app.update();
        app.update();
    }
    assert!(
        matches!(
            &app.world().resource::<LastLoadOutcome>().0,
            Some(LoadOutcome::Success { .. })
        ),
        "ロードは成功するはず: {:?}",
        app.world().resource::<LastLoadOutcome>().0
    );
    // ロードはパネルの開閉状態を初期化する(閉じたパネルの表示は更新されない)ため、開き直して確認する。
    assert!(!app.world().resource::<MilitaryPanelState>().open);
    select(&mut app, &[id]);
    open_panel(&mut app);
    let after = panel_text(&mut app);
    assert!(after.contains("補給(直近の日次補給判定): 35.0%"), "{after}");
    assert!(
        after.contains("装備(MilitaryEquipment): 80 / 100 (充足率 80.0%)"),
        "{after}"
    );
}
