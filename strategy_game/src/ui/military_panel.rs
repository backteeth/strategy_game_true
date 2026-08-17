use crate::app::game_state::GameState;
use crate::common::{ArmyId, CountryId, DivisionDefinitionId, DivisionId, FrontlineId, StateId};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text, t, tf};
use crate::map::division_selection::SelectedDivision;
use crate::map::frontline_selection::{FrontlineAssignmentAttempted, FrontlineSelectMode};
use crate::map::offensive_line_selection::OffensiveLineEditMode;
use crate::military::army::{Army, ArmyRegistry, SelectedArmy};
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::military::recruitment::{
    RecruitFeasibility, evaluate_recruit_feasibility, request_recruitment,
};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::data::WarRegistry;
use crate::war::frontline::{
    ArmyFrontlineAssignFeasibility, FrontlineCommandFeasibility, FrontlineRegistry,
    FrontlineStance, OffensiveLineProgress, compute_offensive_line_progress,
    evaluate_army_frontline_assign_feasibility, evaluate_frontline_division_command_feasibility,
    uncaptured_offensive_line_regions,
};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

/// P21-001: 募兵UIが常に対象とする基本部隊定義。
/// `app/loader.rs`の`spawn_debug_divisions`が初期軍配置に使う`DivisionId(0)`
/// (`assets/data/divisions.ron`の"Standard Infantry")と同一のIDを再利用する。
/// 新規IDは発行しない。
const RECRUIT_DIVISION_ID: DivisionDefinitionId = DivisionDefinitionId(0);

/// 募兵ボタンの背景色(実行可能時)。
const RECRUIT_READY_COLOR: Color = Color::srgba(0.2, 0.5, 0.2, 1.0);
/// 募兵ボタンの背景色(実行不可時・見た目のみ無効化)。
const RECRUIT_DISABLED_COLOR: Color = Color::srgba(0.25, 0.25, 0.28, 1.0);
/// P21-002: 前線命令ボタンの背景色(実行可能時)。
const FRONTLINE_CMD_READY_COLOR: Color = Color::srgba(0.2, 0.35, 0.55, 1.0);
/// P21-002: 前線命令ボタンの背景色(実行不可時)。
const FRONTLINE_CMD_DISABLED_COLOR: Color = RECRUIT_DISABLED_COLOR;
/// P21-002: 現在選択中のスタンスを示すボタンの背景色。
const FRONTLINE_STANCE_ACTIVE_COLOR: Color = Color::srgba(0.15, 0.55, 0.25, 1.0);
/// P21-004: 編成(Army)コマンドボタンの背景色(実行可能時)。
const ARMY_GROUP_CMD_READY_COLOR: Color = Color::srgba(0.5, 0.3, 0.55, 1.0);
/// P21-004: 編成(Army)コマンドボタンの背景色(実行不可時)。
const ARMY_GROUP_CMD_DISABLED_COLOR: Color = RECRUIT_DISABLED_COLOR;

#[derive(Component)]
pub struct MilitaryPanelRoot;

#[derive(Component)]
pub struct MilitaryPanelText;

#[derive(Component)]
pub struct RecruitButton(pub DivisionDefinitionId);

/// P21-001: 募兵ボタンに付随するコスト・実行可否表示のTextマーカー。
#[derive(Component)]
pub struct RecruitInfoText;

/// P21-002: 前線命令ボタン(割当/解除/全解除/停止/防御/攻勢)が発行する命令の種類。
/// キーボード操作(旧`Digit1/2/3/7/8/9`)とボタン操作の両方から、下記`execute_frontline_*`
/// 関数群を共通で呼び出すことでロジックの二重実装を避ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontlineCommand {
    /// 選択中陸軍を前線へ割当(旧Digit1)
    Assign,
    /// 選択中陸軍を前線から解除(旧Digit2)
    Unassign,
    /// 自国プランの全陸軍を解除(旧Digit3)
    UnassignAll,
    /// 自国プランのスタンスを設定(旧Digit7/8/9)
    SetStance(FrontlineStance),
}

#[derive(Component)]
pub struct FrontlineCommandButton(pub FrontlineCommand);

/// P21-002: 前線命令ボタンに付随する実行可否表示のTextマーカー。
#[derive(Component)]
pub struct FrontlineCommandInfoText;

/// P21-004A: 選択中師団の移動命令のみを解除する「移動停止」ボタン。
/// 前線スタンスの「停止」(`FrontlineCommand::SetStance(FrontlineStance::Stopped)`、
/// 国家の前線プラン全体に効く設定で選択に依存しない)とは全く別の操作であり、
/// 個別師団の移動命令(`map::division_selection::stop_division_movement`)のみを対象とする。
#[derive(Component)]
pub struct StopMovementButton;

/// P21-004A: 移動停止ボタンに付随する実行可否表示のTextマーカー。
#[derive(Component)]
pub struct StopMovementInfoText;

/// P21-004: 編成(Army)ボタンが発行する命令の種類。「対象編成」は`SelectedArmy`
/// (編成一覧のクリックで明示的に選ばれた編成)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmyCommand {
    /// 選択中陸軍から新しい編成を作成し、それを選択中編成にする
    Create,
    /// 選択中陸軍を選択中編成へ追加
    AddSelection,
    /// 選択中陸軍を、それぞれの所属編成(あれば)から除外
    RemoveSelection,
    /// 選択中編成を解散
    Disband,
}

#[derive(Component)]
pub struct ArmyCommandButton(pub ArmyCommand);

/// P21-004: 選択中編成の表示のTextマーカー。
#[derive(Component)]
pub struct ArmyStatusText;

/// P21-005: 選択中編成に対する前線割当ボタンが発行する命令の種類。`ArmyCommand`とは
/// 独立した別のコンポーネント型にする(`FrontlineCommandButton`/`StopMovementButton`と
/// 同じく、機能行ごとに専用のボタン型+専用の更新Systemを持たせる既存の慣習を踏襲し、
/// `update_army_ui`の`ArmyCommandButton`クエリと同一エンティティを取り合わないようにする)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmyFrontlineCommand {
    /// 前線選択モードをトグルする(非アクティブ→対象編成でアクティブ化、
    /// 既に対象編成でアクティブ→キャンセル)。
    Assign,
    /// 選択中編成だけを現在の前線割当から解除する。
    Unassign,
}

#[derive(Component)]
pub struct ArmyFrontlineCommandButton(pub ArmyFrontlineCommand);

/// P21-005: 選択中編成の前線割当状況(現在の割当前線/前線選択中/有効な前線がない等)の
/// 表示のTextマーカー。`ArmyStatusText`とは別の行として表示する。
#[derive(Component)]
pub struct ArmyFrontlineStatusText;

/// P21-007: 攻勢線(計画データのみ)の設定/解除/確定/キャンセルボタンが発行する命令の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffensiveLineCommand {
    /// 編集モードをトグルする(非アクティブ→選択中編成でアクティブ化・
    /// 既存の攻勢線があればdraftへ複写、既にアクティブ→キャンセル)。
    StartEdit,
    /// 確認ダイアログなしで即座に攻勢線を解除する。編集中なら編集も同時にキャンセルする。
    Clear,
    /// 編集中のdraftを`FrontlineRegistry::set_offensive_line`へ確定する。
    Confirm,
    /// 編集中のdraftを破棄する。
    Cancel,
}

#[derive(Component)]
pub struct OffensiveLineCommandButton(pub OffensiveLineCommand);

/// P21-007: 攻勢線の現在状態(未選択/未対応/未設定/設定済みN州/編集中M地点)の
/// 表示のTextマーカー。
#[derive(Component)]
pub struct OffensiveLineStatusText;

/// P21-004: 編成一覧を動的に構築する行(Army一覧のクリック可能な各行)の親Node。
/// 更新のたびに子(各編成の行ボタン)を全て破棄・再構築する
/// (`ui::diplomacy_panel`の`DiplomacyContentContainer`と同じパターン)。
#[derive(Component)]
pub struct ArmyListContainer;

/// P21-004: 編成一覧の各行ボタン。クリックするとその編成を選択中編成にし、
/// 所属する生存中の全師団を`SelectedDivision`へ反映する
/// (既存のDivision選択を置き換えるのではなく、一括選択する入口として機能する)。
#[derive(Component)]
pub struct ArmyListRowButton(pub ArmyId);

#[derive(Resource, Default)]
pub struct MilitaryPanelState {
    pub open: bool,
}

#[derive(Component)]
pub struct ToggleMilitaryPanelButton;

pub struct MilitaryPanelPlugin;

impl Plugin for MilitaryPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MilitaryPanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_military_panel)
            .add_systems(
                Update,
                (
                    toggle_military_panel_key,
                    handle_military_panel_scroll,
                    update_military_panel_ui,
                    update_recruit_button_ui,
                    handle_recruit_buttons,
                    update_frontline_command_buttons_ui,
                    handle_frontline_command_buttons,
                    update_army_ui,
                    handle_army_command_buttons,
                    handle_army_list_row_clicks,
                    update_stop_movement_ui,
                    handle_stop_movement_button,
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                (
                    update_army_frontline_ui,
                    handle_army_frontline_command_buttons,
                    handle_frontline_assignment_attempted,
                    cancel_frontline_select_mode_on_panel_close,
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                (
                    update_offensive_line_ui,
                    handle_offensive_line_command_buttons,
                    cancel_offensive_line_edit_on_panel_close,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_military_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    military_registry: Res<MilitaryRegistry>,
) {
    // トグルボタン
    commands
        .spawn((
            ToggleMilitaryPanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(840.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.3, 0.2, 0.5, 0.9)),
        ))
        .with_children(|parent| {
            let (text, marker) =
                localized_text(&catalog, locale.0, "military_panel.toggle_button", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::WHITE),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
        });

    // メインパネル（初期は非表示）
    commands
        .spawn((
            MilitaryPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(75.0),
                width: Val::Px(600.0),
                height: Val::Px(650.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                // P21-004: 編成セクション追加でパネル内容が固定高さ(650px)を超え、
                // 下部が見切れる不具合が発生したため、クリップのみ(clip_y)から
                // マウスホイールでスクロール可能(scroll_y)に変更する。
                overflow: Overflow::scroll_y(),
                ..default()
            },
            ScrollPosition::default(),
            BackgroundColor(Color::srgba(0.08, 0.06, 0.14, 0.95)),
        ))
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "military_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.9, 0.7, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            // P21-001: 募兵セクション
            let (header_text, header_marker) =
                localized_text(&catalog, locale.0, "military_panel.recruit_header", vec![]);
            parent.spawn((
                header_text,
                header_marker,
                TextColor(Color::srgb(0.7, 0.9, 0.7)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            let recruit_unit_name = military_registry
                .definitions
                .get(&RECRUIT_DIVISION_ID)
                .map(|def| def.name.clone())
                .unwrap_or_else(|| "?".to_string());

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    let (btn_text, btn_marker) = localized_text(
                        &catalog,
                        locale.0,
                        "military_panel.recruit_button",
                        vec![("unit", recruit_unit_name)],
                    );
                    row.spawn((
                        RecruitButton(RECRUIT_DIVISION_ID),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(RECRUIT_DISABLED_COLOR),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            btn_text,
                            btn_marker,
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });

                    row.spawn((
                        RecruitInfoText,
                        Text::new(t(
                            &catalog,
                            locale.0,
                            "military_panel.recruit_status_no_selection",
                        )),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                    ));
                });

            // P21-002: 前線命令セクション(旧Digit1/2/3/7/8/9のボタン化)
            let (fl_header_text, fl_header_marker) = localized_text(
                &catalog,
                locale.0,
                "military_panel.frontline_cmd_header",
                vec![],
            );
            parent.spawn((
                fl_header_text,
                fl_header_marker,
                TextColor(Color::srgb(0.6, 0.8, 0.95)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    for (cmd, key) in [
                        (
                            FrontlineCommand::Assign,
                            "military_panel.frontline_assign_button",
                        ),
                        (
                            FrontlineCommand::Unassign,
                            "military_panel.frontline_unassign_button",
                        ),
                        (
                            FrontlineCommand::UnassignAll,
                            "military_panel.frontline_unassign_all_button",
                        ),
                    ] {
                        let (btn_text, btn_marker) =
                            localized_text(&catalog, locale.0, key, vec![]);
                        row.spawn((
                            FrontlineCommandButton(cmd),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(FRONTLINE_CMD_DISABLED_COLOR),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                btn_text,
                                btn_marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });
                    }
                });

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    for (cmd, key) in [
                        (
                            FrontlineCommand::SetStance(FrontlineStance::Stopped),
                            "military_panel.frontline_stop_button",
                        ),
                        (
                            FrontlineCommand::SetStance(FrontlineStance::Defend),
                            "military_panel.frontline_defend_button",
                        ),
                        (
                            FrontlineCommand::SetStance(FrontlineStance::Offensive),
                            "military_panel.frontline_offensive_button",
                        ),
                    ] {
                        let (btn_text, btn_marker) =
                            localized_text(&catalog, locale.0, key, vec![]);
                        row.spawn((
                            FrontlineCommandButton(cmd),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(FRONTLINE_CMD_DISABLED_COLOR),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                btn_text,
                                btn_marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });
                    }

                    row.spawn((
                        FrontlineCommandInfoText,
                        Text::new(t(
                            &catalog,
                            locale.0,
                            "military_panel.frontline_cmd_status_no_frontline",
                        )),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                    ));
                });

            // P21-004A: 選択中師団向けの「移動停止」(前線スタンスの「停止」とは別物)。
            // Army選択で埋まったDivision選択に対しても、それ以外の選択に対しても同じく機能する。
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    let (btn_text, btn_marker) = localized_text(
                        &catalog,
                        locale.0,
                        "military_panel.stop_movement_button",
                        vec![],
                    );
                    row.spawn((
                        StopMovementButton,
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(FRONTLINE_CMD_DISABLED_COLOR),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            btn_text,
                            btn_marker,
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });

                    row.spawn((
                        StopMovementInfoText,
                        Text::new(t(
                            &catalog,
                            locale.0,
                            "military_panel.stop_movement_no_selection",
                        )),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                    ));
                });

            // P21-004: 編成(Army)セクション。下の編成一覧(ArmyListContainer)の各行を
            // クリックすることで「選択中編成」(`SelectedArmy`)を明示的に選べる。
            let (ag_header_text, ag_header_marker) =
                localized_text(&catalog, locale.0, "military_panel.army_header", vec![]);
            parent.spawn((
                ag_header_text,
                ag_header_marker,
                TextColor(Color::srgb(0.85, 0.7, 0.9)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|row| {
                    for (cmd, key) in [
                        (ArmyCommand::Create, "military_panel.army_create_button"),
                        (ArmyCommand::AddSelection, "military_panel.army_add_button"),
                        (
                            ArmyCommand::RemoveSelection,
                            "military_panel.army_remove_button",
                        ),
                        (ArmyCommand::Disband, "military_panel.army_disband_button"),
                    ] {
                        let (btn_text, btn_marker) =
                            localized_text(&catalog, locale.0, key, vec![]);
                        row.spawn((
                            ArmyCommandButton(cmd),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(ARMY_GROUP_CMD_DISABLED_COLOR),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                btn_text,
                                btn_marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });
                    }
                });

            parent.spawn((
                ArmyStatusText,
                Text::new(t(&catalog, locale.0, "military_panel.army_no_target")),
                TextColor(Color::srgb(0.8, 0.8, 0.8)),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
            ));

            // P21-005: 選択中編成の前線割当(設定/解除ボタン+現在の割当状況表示)
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    for (cmd, key) in [
                        (
                            ArmyFrontlineCommand::Assign,
                            "military_panel.army_frontline_assign_button",
                        ),
                        (
                            ArmyFrontlineCommand::Unassign,
                            "military_panel.army_frontline_unassign_button",
                        ),
                    ] {
                        let (btn_text, btn_marker) =
                            localized_text(&catalog, locale.0, key, vec![]);
                        row.spawn((
                            ArmyFrontlineCommandButton(cmd),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(ARMY_GROUP_CMD_DISABLED_COLOR),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                btn_text,
                                btn_marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });
                    }

                    row.spawn((
                        ArmyFrontlineStatusText,
                        Text::new(t(
                            &catalog,
                            locale.0,
                            "military_panel.army_frontline_status_none_selected",
                        )),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                    ));
                });

            // P21-007: 攻勢線(計画データのみ)の設定/解除/確定/キャンセルボタン+状態表示。
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    for (cmd, key) in [
                        (
                            OffensiveLineCommand::StartEdit,
                            "military_panel.offensive_line_start_button",
                        ),
                        (
                            OffensiveLineCommand::Clear,
                            "military_panel.offensive_line_clear_button",
                        ),
                        (
                            OffensiveLineCommand::Confirm,
                            "military_panel.offensive_line_confirm_button",
                        ),
                        (
                            OffensiveLineCommand::Cancel,
                            "military_panel.offensive_line_cancel_button",
                        ),
                    ] {
                        let (btn_text, btn_marker) =
                            localized_text(&catalog, locale.0, key, vec![]);
                        row.spawn((
                            OffensiveLineCommandButton(cmd),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(ARMY_GROUP_CMD_DISABLED_COLOR),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                btn_text,
                                btn_marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });
                    }

                    row.spawn((
                        OffensiveLineStatusText,
                        Text::new(t(
                            &catalog,
                            locale.0,
                            "military_panel.offensive_line_status_none_selected",
                        )),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextLayout {
                            linebreak: LineBreak::AnyCharacter,
                            ..default()
                        },
                    ));
                });

            // P21-004: 編成一覧本体はクリック可能な行として動的に構築される
            // (`update_army_ui`が更新のたびに子を全破棄・再構築する)。
            // 初期状態(0件)のプレースホルダ行はここで1つだけ用意しておく。
            parent
                .spawn((
                    ArmyListContainer,
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    list.spawn((
                        Text::new(t(&catalog, locale.0, "military_panel.army_list_empty")),
                        TextColor(Color::srgb(0.85, 0.85, 0.85)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                });

            // パネル内テキスト
            // NOTE: このパネルは複数の翻訳キーを1行ずつ結合した合成テキストであり、
            // 単一の翻訳キーで表現できないため、意図的に`LocalizedText`マーカーを付与しない
            // (汎用の`retranslate_on_locale_change`による上書きを避ける)。
            // 言語切り替え時の再翻訳は`update_military_panel_ui`自身が
            // `!state.open && !locale.is_changed()`ガードにより担う。
            parent.spawn((
                MilitaryPanelText,
                Text::new(t(&catalog, locale.0, "military_panel.loading")),
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
            ));
        });
}

fn toggle_military_panel_key(
    mut state: ResMut<MilitaryPanelState>,
    mut active_panel: ResMut<crate::ui::ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<ToggleMilitaryPanelButton>, Changed<Interaction>)>,
    mut panel_q: Query<&mut Node, With<MilitaryPanelRoot>>,
) {
    let mut toggle = false;
    if keys.just_pressed(KeyCode::KeyM) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        active_panel.toggle(crate::ui::PanelKind::Military);
        state.open = active_panel.current == crate::ui::PanelKind::Military;
        if let Ok(mut node) = panel_q.single_mut() {
            node.display = if state.open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

/// P21-004: 軍事パネルの内容が固定高さを超える場合にマウスホイールでスクロールする。
/// パネル自身の`Node`に`overflow: Overflow::scroll_y()`+`ScrollPosition`が設定済み。
/// 上端でのクランプのみ行い、下端は厳密な最大値計算を避けて緩めの上限に留める
/// (陸軍・編成・戦闘の行数は可変で、正確な最大値はレイアウト計算後でないと分からないため)。
fn handle_military_panel_scroll(
    state: Res<MilitaryPanelState>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut panel_q: Query<&mut ScrollPosition, With<MilitaryPanelRoot>>,
) {
    if !state.open {
        scroll_events.clear();
        return;
    }

    let mut delta_y = 0.0_f32;
    for event in scroll_events.read() {
        delta_y += match event.unit {
            MouseScrollUnit::Line => event.y * 24.0,
            MouseScrollUnit::Pixel => event.y,
        };
    }
    if delta_y == 0.0 {
        return;
    }

    if let Ok(mut scroll) = panel_q.single_mut() {
        scroll.y = (scroll.y - delta_y).clamp(0.0, 4000.0);
    }
}

fn division_status_key(status: DivisionStatus) -> &'static str {
    match status {
        DivisionStatus::Idle => "division_status.idle",
        DivisionStatus::Moving => "division_status.moving",
        DivisionStatus::Fighting => "division_status.fighting",
        DivisionStatus::Occupying => "division_status.occupying",
        DivisionStatus::Retreating => "division_status.retreating",
        DivisionStatus::Disbanding => "division_status.disbanding",
        DivisionStatus::Destroyed => "division_status.destroyed",
    }
}

/// P21-001: 募兵ボタンのコスト・実行可否表示を更新する。
/// `update_military_panel_ui`はBevyのSystemParamタプル実装の引数数上限に近いため、
/// 既存関数へ引数を追加せず独立したSystemとして切り出す。
#[allow(clippy::too_many_arguments)]
fn update_recruit_button_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    selected_state: Res<SelectedState>,
    loc: crate::localization::Loc,
    mut recruit_btn_q: Query<&mut BackgroundColor, With<RecruitButton>>,
    mut recruit_text_q: Query<&mut Text, With<RecruitInfoText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get(player_cid) else {
        return;
    };

    let feasibility = evaluate_recruit_feasibility(
        selected_state.0,
        player_cid,
        &state_registry,
        country,
        &military_registry,
        RECRUIT_DIVISION_ID,
    );
    let recruit_status_key = match feasibility {
        RecruitFeasibility::Ready => "military_panel.recruit_status_ready",
        RecruitFeasibility::NoStateSelected => "military_panel.recruit_status_no_selection",
        RecruitFeasibility::NotOwnState => "military_panel.recruit_status_not_owned",
        RecruitFeasibility::DefinitionUnavailable => "military_panel.recruit_status_no_definition",
        RecruitFeasibility::InsufficientManpower => {
            "military_panel.recruit_status_insufficient_manpower"
        }
        RecruitFeasibility::InsufficientFunds => "military_panel.recruit_status_insufficient_funds",
    };
    if let Some(def) = military_registry.definitions.get(&RECRUIT_DIVISION_ID) {
        let recruit_line = tf(
            catalog,
            locale.0,
            "military_panel.recruit_cost",
            vec![
                ("avail_manpower", country.available_manpower.to_string()),
                ("req_manpower", def.required_manpower.to_string()),
                ("avail_treasury", format!("{:.0}", country.treasury)),
                ("req_treasury", format!("{:.0}", def.required_equipment)),
                ("status", t(catalog, locale.0, recruit_status_key)),
            ],
        );
        if let Ok(mut recruit_text) = recruit_text_q.single_mut()
            && recruit_text.0 != recruit_line
        {
            recruit_text.0 = recruit_line;
        }
    }
    for mut bg in recruit_btn_q.iter_mut() {
        *bg = BackgroundColor(if feasibility.is_ready() {
            RECRUIT_READY_COLOR
        } else {
            RECRUIT_DISABLED_COLOR
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn update_military_panel_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    battle_registry: Res<BattleRegistry>,
    mut frontline_registry: ResMut<crate::war::frontline::FrontlineRegistry>,
    ai_registry: Res<crate::war::military_ai::MilitaryAiRegistry>,
    country_ai_registry: Res<crate::country::country_ai::CountryAiRegistry>,
    frontline_settings: Res<crate::map::frontline_render::FrontlineRenderSettings>,
    selected_division: Res<SelectedDivision>,
    keys: Res<ButtonInput<KeyCode>>,
    loc: crate::localization::Loc,
    mut text_q: Query<&mut Text, With<MilitaryPanelText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };
    let Some(country) = country_registry.get(player_cid) else {
        return;
    };

    let tr = |key: &'static str| t(catalog, locale.0, key);
    let trf =
        |key: &'static str, args: Vec<(&'static str, String)>| tf(catalog, locale.0, key, args);

    // プレイヤー参加中のアクティブ戦争と前線を取得
    let active_war = war_registry.get_active_war_for_country(player_cid);
    let frontline =
        active_war.and_then(|w| frontline_registry.get_frontline_for_war(w.id).cloned());

    // --- キー操作による前線コントロール ---
    // P21-002: 実際の副作用は`execute_frontline_*`関数群に切り出し、下記の
    // `handle_frontline_command_buttons`(ボタン操作)と完全に同じ処理を呼び出す。
    // ロジックをキー処理内に直書きしない。
    if let Some(ref fl) = frontline {
        // [Key1] 選択中陸軍(複数可)を前線へ割り当て
        if keys.just_pressed(KeyCode::Digit1) {
            execute_frontline_assign(
                &mut frontline_registry,
                &military_registry,
                &war_registry,
                &selected_division.sorted_ids(),
                fl.frontline_id,
                player_cid,
            );
        }
        // [Key2] 選択中陸軍(複数可)を前線から解除
        if keys.just_pressed(KeyCode::Digit2) {
            execute_frontline_unassign(
                &mut frontline_registry,
                &military_registry,
                &selected_division.sorted_ids(),
                player_cid,
            );
        }
        // [Key3] 全部隊の割り当て解除
        if keys.just_pressed(KeyCode::Digit3) {
            frontline_registry.unassign_all_divisions_for_plan(fl.frontline_id, player_cid);
        }
        // [Key7] 停止 (Stopped)
        if keys.just_pressed(KeyCode::Digit7) {
            execute_frontline_set_stance(
                &mut frontline_registry,
                fl.frontline_id,
                player_cid,
                FrontlineStance::Stopped,
            );
        }
        // [Key8] 防御 (Defend)
        if keys.just_pressed(KeyCode::Digit8) {
            execute_frontline_set_stance(
                &mut frontline_registry,
                fl.frontline_id,
                player_cid,
                FrontlineStance::Defend,
            );
        }
        // [Key9] 攻勢 (Offensive)
        if keys.just_pressed(KeyCode::Digit9) {
            execute_frontline_set_stance(
                &mut frontline_registry,
                fl.frontline_id,
                player_cid,
                FrontlineStance::Offensive,
            );
        }
    }

    // 自国の軍隊を集計
    let my_divisions: Vec<_> = military_registry
        .divisions
        .values()
        .filter(|a| a.owner == player_cid)
        .collect();

    let mut lines = Vec::new();

    lines.push(trf(
        "military_panel.manpower",
        vec![
            ("available", country.available_manpower.to_string()),
            ("mobilized", country.mobilized_manpower.to_string()),
        ],
    ));
    lines.push(trf(
        "military_panel.upkeep",
        vec![("cost", format!("{:.1}", country.monthly_military_expenses))],
    ));
    lines.push(trf(
        "military_panel.frontline_visibility",
        vec![(
            "state",
            tr(if frontline_settings.visible {
                "common.on"
            } else {
                "common.off"
            }),
        )],
    ));
    lines.push(String::new());

    // 前線・作戦命令表示
    lines.push(tr("military_panel.frontline_orders_header"));
    if let (Some(war), Some(fl)) = (active_war, frontline.as_ref()) {
        let is_attacker = war.attackers.contains(&player_cid);
        let atk_name = country_registry
            .get(fl.attacker_country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let def_name = country_registry
            .get(fl.defender_country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));

        lines.push(trf(
            "military_panel.frontline_id",
            vec![
                ("id", fl.frontline_id.0.to_string()),
                ("war", war.name.clone()),
            ],
        ));
        lines.push(trf(
            "military_panel.frontline_belligerents",
            vec![
                ("attacker", atk_name),
                ("defender", def_name),
                ("pairs", fl.border_region_pairs.len().to_string()),
            ],
        ));
        lines.push(trf(
            "military_panel.frontline_regions",
            vec![
                (
                    "own",
                    if is_attacker {
                        fl.attacker_front_regions.len()
                    } else {
                        fl.defender_front_regions.len()
                    }
                    .to_string(),
                ),
                (
                    "enemy",
                    if is_attacker {
                        fl.defender_front_regions.len()
                    } else {
                        fl.attacker_front_regions.len()
                    }
                    .to_string(),
                ),
            ],
        ));

        if fl.border_region_pairs.is_empty() {
            lines.push(tr("military_panel.frontline_no_border"));
        }

        let plan = frontline_registry.get_plan(fl.frontline_id, player_cid);
        let stance = plan.map(|p| p.stance).unwrap_or_default();
        let assigned_ids = plan
            .map(|p| p.assigned_division_ids.as_slice())
            .unwrap_or(&[]);

        lines.push(trf(
            "military_panel.order_state",
            vec![("stance", tr(stance.display_name()))],
        ));

        if let Some(obj_id) = plan.and_then(|p| p.objective_region_id) {
            let obj_name = state_registry
                .get(obj_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.objective_set",
                vec![("state", obj_name), ("id", obj_id.0.to_string())],
            ));
        } else {
            lines.push(tr("military_panel.objective_unset"));
        }

        // 割当部隊の内訳集計
        let mut count_idle = 0;
        let mut count_moving = 0;
        let mut count_fighting = 0;
        for &id in assigned_ids {
            if let Some(a) = military_registry.divisions.get(&id) {
                match a.status {
                    DivisionStatus::Idle => count_idle += 1,
                    DivisionStatus::Moving => count_moving += 1,
                    DivisionStatus::Fighting => count_fighting += 1,
                    _ => {}
                }
            }
        }
        lines.push(trf(
            "military_panel.assigned_units",
            vec![
                ("total", assigned_ids.len().to_string()),
                ("idle", count_idle.to_string()),
                ("moving", count_moving.to_string()),
                ("fighting", count_fighting.to_string()),
            ],
        ));

        lines.push(tr("military_panel.controls_hint"));
    } else {
        lines.push(tr("military_panel.no_active_war"));
    }
    lines.push(String::new());

    // AI軍事作戦状況
    if !ai_registry.ai_states.is_empty() {
        lines.push(tr("military_panel.ai_ops_header"));
        let mut ai_countries: Vec<_> = ai_registry.ai_states.values().collect();
        ai_countries.sort_by_key(|a| a.country_id.0);

        for ai in ai_countries {
            let country_name = country_registry
                .get(ai.country_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.ai_ops_line",
                vec![
                    ("country", country_name),
                    ("own_power", ai.estimated_own_power.to_string()),
                    ("enemy_power", ai.estimated_enemy_power.to_string()),
                    ("reason", tr(ai.last_decision_reason.display_name())),
                ],
            ));
        }
        lines.push(String::new());
    }

    // 国家AI運営状況
    if !country_ai_registry.ai_states.is_empty() {
        lines.push(tr("military_panel.country_ai_header"));
        let mut country_ais: Vec<_> = country_ai_registry.ai_states.values().collect();
        country_ais.sort_by_key(|c| c.country_id.0);

        for cai in country_ais {
            let country_name = country_registry
                .get(cai.country_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.country_ai_line",
                vec![
                    ("country", country_name),
                    ("mode", tr(cai.mode.display_name())),
                    ("reason", tr(cai.decision_reason.display_name())),
                ],
            ));
        }
        lines.push(String::new());
    }

    // 選択中ユニット詳細(複数選択時は簡易一覧、単一選択時は従来通り詳細表示)
    if selected_division.len() > 1 {
        lines.push(trf(
            "military_panel.multi_selected_header",
            vec![("count", selected_division.len().to_string())],
        ));
        for division_id in selected_division.sorted_ids() {
            if let Some(division) = military_registry.divisions.get(&division_id) {
                let state_name = state_registry
                    .get(division.current_state)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| tr("common.unknown"));
                lines.push(trf(
                    "military_panel.multi_selected_line",
                    vec![
                        ("id", division.id.0.to_string()),
                        ("state", state_name),
                        ("status", tr(division_status_key(division.status))),
                    ],
                ));
            }
        }
        lines.push(String::new());
    } else if let Some(division) = selected_division
        .primary()
        .and_then(|id| military_registry.divisions.get(&id))
    {
        lines.push(tr("military_panel.selected_unit_header"));
        let owner_name = country_registry
            .get(division.owner)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let current_state_name = state_registry
            .get(division.current_state)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        lines.push(trf(
            "military_panel.selected_unit_id",
            vec![("id", division.id.0.to_string()), ("owner", owner_name)],
        ));
        lines.push(trf(
            "military_panel.selected_unit_location",
            vec![("state", current_state_name)],
        ));

        // 前線割り当て状態
        if let Some(fl_id) = frontline_registry.division_frontline_map.get(&division.id) {
            lines.push(trf(
                "military_panel.frontline_assigned",
                vec![("id", fl_id.0.to_string())],
            ));
        } else {
            lines.push(tr("military_panel.frontline_unassigned"));
        }

        // 割り当て不可理由の判定と表示
        if division.owner != player_cid {
            lines.push(tr("military_panel.not_own_division"));
        } else if active_war.is_none() {
            lines.push(tr("military_panel.no_active_war_assign"));
        } else if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
            lines.push(tr("military_panel.destroyed_or_no_power"));
        }

        // 戦力・組織率
        lines.push(trf(
            "military_panel.strength",
            vec![
                ("current", division.manpower.to_string()),
                ("max", division.max_manpower.to_string()),
                (
                    "percent",
                    format!(
                        "{:.0}",
                        division.manpower as f32 / division.max_manpower as f32 * 100.0
                    ),
                ),
            ],
        ));
        lines.push(trf(
            "military_panel.organization",
            vec![
                ("current", format!("{:.0}", division.organization)),
                ("max", format!("{:.0}", division.max_organization)),
                (
                    "percent",
                    format!(
                        "{:.0}",
                        division.organization / division.max_organization * 100.0
                    ),
                ),
            ],
        ));
        lines.push(trf(
            "military_panel.power",
            vec![
                ("attack", division.attack_power.to_string()),
                ("defense", division.defense_power.to_string()),
            ],
        ));

        lines.push(trf(
            "military_panel.status_line",
            vec![("status", tr(division_status_key(division.status)))],
        ));

        if let Some(dest_id) = division.destination {
            let dest_name = state_registry
                .get(dest_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| tr("common.unknown"));
            lines.push(trf(
                "military_panel.destination_set",
                vec![("state", dest_name)],
            ));
        } else {
            lines.push(tr("military_panel.destination_none"));
        }
        lines.push(String::new());
    }

    // 進行中の戦闘一覧
    let ongoing_battles: Vec<_> = battle_registry
        .battles
        .values()
        .filter(|b| b.status == BattleStatus::Ongoing)
        .collect();

    if !ongoing_battles.is_empty() {
        lines.push(trf(
            "military_panel.ongoing_battles_header",
            vec![("count", ongoing_battles.len().to_string())],
        ));
        for battle in &ongoing_battles {
            let battle_state_name = state_registry
                .get(battle.state_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let atk_name = country_registry
                .get(battle.attacker_country)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let def_name = country_registry
                .get(battle.defender_country)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".to_string());

            lines.push(trf(
                "military_panel.battle_line",
                vec![
                    ("state", battle_state_name),
                    ("attacker", atk_name),
                    ("defender", def_name),
                ],
            ));
        }
        lines.push(String::new());
    }

    lines.push(trf(
        "military_panel.division_list_header",
        vec![("count", my_divisions.len().to_string())],
    ));

    for division in &my_divisions {
        let state_name = state_registry
            .get(division.current_state)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| tr("common.unknown"));
        let status_str = tr(division_status_key(division.status));
        let fl_tag = if frontline_registry
            .division_frontline_map
            .contains_key(&division.id)
        {
            tr("military_panel.frontline_tag")
        } else {
            String::new()
        };
        let selected = selected_division.is_selected(division.id);
        let sel_mark = if selected { "► " } else { "  " };
        lines.push(trf(
            "military_panel.division_line",
            vec![
                ("mark", sel_mark.to_string()),
                ("id", division.id.0.to_string()),
                ("state", state_name),
                ("status", status_str),
                ("frontline_tag", fl_tag),
                ("manpower", division.manpower.to_string()),
            ],
        ));
    }

    if let Ok(mut text) = text_q.single_mut() {
        let joined = lines.join("\n");
        if text.0 != joined {
            text.0 = joined;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_recruit_buttons(
    btn_q: Query<(&Interaction, &RecruitButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    selected_state: Res<SelectedState>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let def_id = btn.0;

        // 押下時点で全条件を再評価する(表示更新との1フレームのズレを許容しない)。
        // Readyでなければ見た目上の無効ボタンとして何もしない(部分更新も起こさない)。
        let Some(country) = country_registry.get(player_cid) else {
            continue;
        };
        let feasibility = evaluate_recruit_feasibility(
            selected_state.0,
            player_cid,
            &state_registry,
            country,
            &military_registry,
            def_id,
        );
        if !feasibility.is_ready() {
            continue;
        }

        let Some(target_state) = selected_state.0 else {
            continue;
        };
        let Some(country_mut) = country_registry.get_mut(player_cid) else {
            continue;
        };

        // request_recruitment が資金・人的資源・部隊定義を原子的に再検証して実行する。
        if request_recruitment(country_mut, &military_registry, def_id, target_state).is_ok() {
            let unit_name = military_registry
                .definitions
                .get(&def_id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let state_name = state_registry
                .get(target_state)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
            notif_writer.write(GameNotification {
                message: tf(
                    &catalog,
                    locale.0,
                    "military_panel.recruit_queued",
                    vec![("unit", unit_name), ("state", state_name)],
                ),
            });
        }
        // Err の場合: 直前のevaluate_recruit_feasibilityと矛盾する状態変化が
        // 同一フレーム内に起きた場合のみ発生しうるが、request_recruitment自体が
        // 検証失敗時は一切のフィールドを変更しないため部分更新は起こらない。
    }
}

/// P21-002: 選択中陸軍を前線へ割り当てる。`FrontlineAssignButton`のクリックと
/// `Digit1`キーの両方から呼ばれる唯一の実行経路(ロジックの二重実装を避ける)。
/// `assign_division`自身が所有者・戦争状態・撃破済みかを再検証するため、ここでは
/// 追加の検証を行わない。
fn execute_frontline_assign(
    frontline_registry: &mut FrontlineRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &WarRegistry,
    selected_division_ids: &[DivisionId],
    frontline_id: FrontlineId,
    player_cid: CountryId,
) {
    // 選択中の各陸軍へ独立に割当を試みる。1個の失敗(所有者不一致・撃破済み等)は
    // 他の陸軍の割当を妨げない(P21-003の複数選択調査で決めた方針)。
    for &division_id in selected_division_ids {
        let _ = frontline_registry.assign_division(
            division_id,
            frontline_id,
            player_cid,
            military_registry,
            war_registry,
        );
    }
}

/// P21-002: 選択中陸軍を前線から解除する。`FrontlineUnassignButton`のクリックと
/// `Digit2`キーの両方から呼ばれる唯一の実行経路。`unassign_division`が所有者を再検証する
/// ため、選択中陸軍が他国のものであっても(選択自体は所有者不問のため起こり得る)、
/// ここで解除が実行されることはない。
fn execute_frontline_unassign(
    frontline_registry: &mut FrontlineRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    for &division_id in selected_division_ids {
        let _ = frontline_registry.unassign_division(division_id, player_cid, military_registry);
    }
}

/// P21-002: 自国プランのスタンスを設定する。`FrontlineCommandButton(SetStance(_))`の
/// クリックと`Digit7/8/9`キーの両方から呼ばれる唯一の実行経路。
fn execute_frontline_set_stance(
    frontline_registry: &mut FrontlineRegistry,
    frontline_id: FrontlineId,
    player_cid: CountryId,
    stance: FrontlineStance,
) {
    if let Some(plan) = frontline_registry.get_plan_mut(frontline_id, player_cid) {
        plan.stance = stance;
    }
}

/// P21-002: 前線命令ボタンの背景色・実行可否テキストを更新する。
/// `update_recruit_button_ui`と同じ理由(SystemParamタプル引数数上限)で独立System化。
#[allow(clippy::too_many_arguments)]
fn update_frontline_command_buttons_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    war_registry: Res<WarRegistry>,
    frontline_registry: Res<FrontlineRegistry>,
    selected_division: Res<SelectedDivision>,
    loc: crate::localization::Loc,
    mut btn_q: Query<(&FrontlineCommandButton, &mut BackgroundColor)>,
    mut info_text_q: Query<&mut Text, With<FrontlineCommandInfoText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let active_war = war_registry.get_active_war_for_country(player_cid);
    let frontline = active_war.and_then(|w| frontline_registry.get_frontline_for_war(w.id));
    let current_stance = frontline.and_then(|fl| {
        frontline_registry
            .get_plan(fl.frontline_id, player_cid)
            .map(|p| p.stance)
    });

    let division_feasibility = evaluate_frontline_division_command_feasibility(
        &selected_division.sorted_ids(),
        player_cid,
        &military_registry,
        frontline,
    );

    for (btn, mut bg) in btn_q.iter_mut() {
        *bg = BackgroundColor(match btn.0 {
            FrontlineCommand::Assign | FrontlineCommand::Unassign => {
                if division_feasibility.is_ready() {
                    FRONTLINE_CMD_READY_COLOR
                } else {
                    FRONTLINE_CMD_DISABLED_COLOR
                }
            }
            FrontlineCommand::UnassignAll => {
                if frontline.is_some() {
                    FRONTLINE_CMD_READY_COLOR
                } else {
                    FRONTLINE_CMD_DISABLED_COLOR
                }
            }
            FrontlineCommand::SetStance(stance) => {
                if frontline.is_none() {
                    FRONTLINE_CMD_DISABLED_COLOR
                } else if current_stance == Some(stance) {
                    FRONTLINE_STANCE_ACTIVE_COLOR
                } else {
                    FRONTLINE_CMD_READY_COLOR
                }
            }
        });
    }

    let status_key = if frontline.is_none() {
        "military_panel.frontline_cmd_status_no_frontline"
    } else {
        match division_feasibility {
            FrontlineCommandFeasibility::Ready => "military_panel.frontline_cmd_status_ready",
            FrontlineCommandFeasibility::NoDivisionSelected
            | FrontlineCommandFeasibility::DivisionNotFound => {
                "military_panel.frontline_cmd_status_no_division"
            }
            FrontlineCommandFeasibility::NotOwnDivision => {
                "military_panel.frontline_cmd_status_not_own_division"
            }
            FrontlineCommandFeasibility::DivisionDestroyed => {
                "military_panel.frontline_cmd_status_division_destroyed"
            }
            FrontlineCommandFeasibility::NoActiveFrontline => {
                "military_panel.frontline_cmd_status_no_frontline"
            }
        }
    };
    if let Ok(mut text) = info_text_q.single_mut() {
        let rendered = t(catalog, locale.0, status_key);
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
}

/// P21-002: 前線命令ボタン(割当/解除/全解除/停止/防御/攻勢)のクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ命令を発行する
/// (募兵・講和ボタンと同型のパターン)。実行対象は常に「プレイヤーが参加する
/// アクティブな戦争の前線」に固定され、対象州クリックのような追加入力は不要。
fn handle_frontline_command_buttons(
    btn_q: Query<(&Interaction, &FrontlineCommandButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    war_registry: Res<WarRegistry>,
    military_registry: Res<MilitaryRegistry>,
    selected_division: Res<SelectedDivision>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };
    let Some(active_war) = war_registry.get_active_war_for_country(player_cid) else {
        return;
    };
    let Some(frontline_id) = frontline_registry
        .get_frontline_for_war(active_war.id)
        .map(|fl| fl.frontline_id)
    else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            FrontlineCommand::Assign => execute_frontline_assign(
                &mut frontline_registry,
                &military_registry,
                &war_registry,
                &selected_division.sorted_ids(),
                frontline_id,
                player_cid,
            ),
            FrontlineCommand::Unassign => execute_frontline_unassign(
                &mut frontline_registry,
                &military_registry,
                &selected_division.sorted_ids(),
                player_cid,
            ),
            FrontlineCommand::UnassignAll => {
                frontline_registry.unassign_all_divisions_for_plan(frontline_id, player_cid);
            }
            FrontlineCommand::SetStance(stance) => execute_frontline_set_stance(
                &mut frontline_registry,
                frontline_id,
                player_cid,
                stance,
            ),
        }
    }
}

/// P21-004A: 選択中の自国師団すべてについて移動命令を解除する(「移動停止」)。
/// 他国師団は`stop_division_movement`自身が無視する。
fn execute_stop_movement(
    military_registry: &mut MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    for &division_id in selected_division_ids {
        crate::map::division_selection::stop_division_movement(
            division_id,
            player_cid,
            military_registry,
        );
    }
}

/// P21-004A: 移動停止ボタンの背景色・実行可否テキストを更新する。
#[allow(clippy::too_many_arguments)]
fn update_stop_movement_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    selected_division: Res<SelectedDivision>,
    loc: crate::localization::Loc,
    mut btn_q: Query<&mut BackgroundColor, With<StopMovementButton>>,
    mut info_text_q: Query<&mut Text, With<StopMovementInfoText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let own_count = selected_division
        .sorted_ids()
        .iter()
        .filter(|id| {
            military_registry
                .divisions
                .get(id)
                .map(|d| d.owner == player_cid)
                .unwrap_or(false)
        })
        .count();
    let can_stop = own_count > 0;

    if let Ok(mut bg) = btn_q.single_mut() {
        *bg = BackgroundColor(if can_stop {
            FRONTLINE_CMD_READY_COLOR
        } else {
            FRONTLINE_CMD_DISABLED_COLOR
        });
    }

    let text = if can_stop {
        tf(
            catalog,
            locale.0,
            "military_panel.stop_movement_target_line",
            vec![("count", own_count.to_string())],
        )
    } else {
        t(
            catalog,
            locale.0,
            "military_panel.stop_movement_no_selection",
        )
    };
    if let Ok(mut text_component) = info_text_q.single_mut()
        && text_component.0 != text
    {
        text_component.0 = text;
    }
}

/// P21-004A: 「移動停止」ボタンのクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ命令を発行する。
fn handle_stop_movement_button(
    btn_q: Query<&Interaction, (With<StopMovementButton>, Changed<Interaction>)>,
    player_country: Res<PlayerCountry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    selected_division: Res<SelectedDivision>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for interaction in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let selected_ids = selected_division.sorted_ids();
        execute_stop_movement(&mut military_registry, &selected_ids, player_cid);
    }
}

/// P21-004: 選択中陸軍から新しい編成を作成し、それを選択中編成にする
/// (「Army作成後、そのArmyを選択状態にする」仕様)。所有者不一致・撃破済み陸軍は
/// `ArmyRegistry::create_army`が黙って除外する。
fn execute_army_create(
    army_registry: &mut ArmyRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
    selected_army: &mut SelectedArmy,
) {
    if let Some(new_id) =
        army_registry.create_army(player_cid, selected_division_ids, military_registry)
    {
        selected_army.0 = Some(new_id);
    }
}

/// P21-004: 選択中陸軍を選択中編成(`SelectedArmy`)へ追加する。
fn execute_army_add_selection(
    army_registry: &mut ArmyRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
    selected_army: SelectedArmy,
) {
    let Some(target) = selected_army.0 else {
        return;
    };
    for &division_id in selected_division_ids {
        let _ = army_registry.add_division(target, division_id, player_cid, military_registry);
    }
}

/// P21-004: 選択中陸軍を、それぞれの所属編成(あれば)から除外する(未所属へ戻す)。
/// 対象は選択中編成に限らず、選択中の各陸軍が実際に所属している編成ごとに判定する
/// (複数編成にまたがる選択でも一括で「未所属に戻す」操作として機能する)。
fn execute_army_remove_selection(
    army_registry: &mut ArmyRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    for &division_id in selected_division_ids {
        let _ = army_registry.remove_division(division_id, player_cid, military_registry);
    }
}

/// P21-004: 選択中編成を解散する。所属していた師団は全員未所属へ戻る。
/// 解散に成功した場合は選択中編成をNoneへ戻す(「解散したArmyの選択状態を解除」仕様)。
fn execute_army_disband(
    army_registry: &mut ArmyRegistry,
    player_cid: CountryId,
    selected_army: &mut SelectedArmy,
) {
    let Some(target) = selected_army.0 else {
        return;
    };
    if army_registry.disband(target, player_cid).is_ok() {
        selected_army.0 = None;
    }
}

/// P21-005: 選択中編成に対して前線選択モードをトグルする。既に対象編成でモードが
/// アクティブなら解除(キャンセル)し、そうでなければ対象編成でアクティブ化する。
/// 割当自体はこの時点では一切行わない(モードへ入る/出るだけ)。
fn execute_army_frontline_assign_toggle(
    selected_army: SelectedArmy,
    mode: &mut FrontlineSelectMode,
) {
    let Some(army_id) = selected_army.0 else {
        return;
    };
    if mode.army_id == Some(army_id) {
        mode.cancel();
    } else {
        mode.activate(army_id);
    }
}

/// P21-005: 選択中編成だけを現在の前線割当から解除する。成功したかを返す(通知表示用)。
/// Divisionの現在地・移動先・経路・戦闘状態は一切変更しない(`unassign_army`自身の保証)。
fn execute_army_frontline_unassign(
    frontline_registry: &mut FrontlineRegistry,
    army_registry: &ArmyRegistry,
    selected_army: SelectedArmy,
    player_cid: CountryId,
) -> bool {
    let Some(army_id) = selected_army.0 else {
        return false;
    };
    frontline_registry
        .unassign_army(army_id, player_cid, army_registry)
        .is_ok()
}

/// P21-004: 編成一覧の行をクリックした際の処理。選択中編成をクリックした編成へ切り替え、
/// その所属する生存中の全師団をDivision選択へ反映する。既存のDivision選択を置き換える
/// のではなく、一括選択する「入口」として機能する(選択後は通常のDivision選択と全く
/// 同じ経路で一括移動・停止・接敵戦闘を利用できる)。他国編成のクリックは無視する。
fn execute_army_list_row_click(
    army_registry: &ArmyRegistry,
    selected_division: &mut SelectedDivision,
    selected_army: &mut SelectedArmy,
    clicked_id: ArmyId,
    player_cid: CountryId,
) {
    let Some(army) = army_registry.armies.get(&clicked_id) else {
        return;
    };
    if army.owner != player_cid {
        return;
    }
    selected_army.0 = Some(clicked_id);
    selected_division.select_only_many(army.member_division_ids.iter().copied());
}

/// P21-004: 編成コマンドボタンの背景色・選択中編成表示・編成一覧(クリック可能な行)を
/// 更新する。`update_military_panel_ui`はSystemParamタプル引数数上限に近いため
/// (`update_recruit_button_ui`/`update_frontline_command_buttons_ui`と同じ理由)、
/// 独立したSystemとして切り出す。
#[allow(clippy::too_many_arguments)]
fn update_army_ui(
    mut commands: Commands,
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    army_registry: Res<ArmyRegistry>,
    selected_division: Res<SelectedDivision>,
    mut selected_army: ResMut<SelectedArmy>,
    loc: crate::localization::Loc,
    mut btn_q: Query<(&ArmyCommandButton, &mut BackgroundColor)>,
    mut status_text_q: Query<&mut Text, With<ArmyStatusText>>,
    list_container_q: Query<(Entity, Option<&Children>), With<ArmyListContainer>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let tr = |key: &'static str| t(catalog, locale.0, key);
    let trf =
        |key: &'static str, args: Vec<(&'static str, String)>| tf(catalog, locale.0, key, args);

    // 選択中編成の再検証: 解散・自動解散で消滅した編成、他国編成を参照し続けない
    if let Some(id) = selected_army.0
        && army_registry.armies.get(&id).map(|a| a.owner) != Some(player_cid)
    {
        selected_army.0 = None;
    }

    let selected_ids = selected_division.sorted_ids();
    let is_own = |id: &DivisionId| {
        military_registry
            .divisions
            .get(id)
            .map(|a| a.owner == player_cid)
            .unwrap_or(false)
    };
    let has_own_selection = selected_ids.iter().any(is_own);
    let target_army = selected_army.0;
    let has_selection_outside_target = selected_ids
        .iter()
        .any(|id| is_own(id) && army_registry.army_for_division(*id) != target_army);
    let has_grouped_selection = selected_ids
        .iter()
        .any(|id| army_registry.army_for_division(*id).is_some());

    let can_create = has_own_selection;
    let can_add = target_army.is_some() && has_selection_outside_target;
    let can_remove = has_grouped_selection;
    let can_disband = target_army.is_some();

    for (btn, mut bg) in btn_q.iter_mut() {
        let ready = match btn.0 {
            ArmyCommand::Create => can_create,
            ArmyCommand::AddSelection => can_add,
            ArmyCommand::RemoveSelection => can_remove,
            ArmyCommand::Disband => can_disband,
        };
        *bg = BackgroundColor(if ready {
            ARMY_GROUP_CMD_READY_COLOR
        } else {
            ARMY_GROUP_CMD_DISABLED_COLOR
        });
    }

    let status_line = target_army
        .and_then(|id| army_registry.armies.get(&id))
        .map(|group| {
            trf(
                "military_panel.army_target_line",
                vec![
                    ("name", group.name.clone()),
                    ("count", group.member_division_ids.len().to_string()),
                ],
            )
        })
        .unwrap_or_else(|| tr("military_panel.army_no_target"));
    if let Ok(mut text) = status_text_q.single_mut()
        && text.0 != status_line
    {
        text.0 = status_line;
    }

    let Ok((container_entity, children_opt)) = list_container_q.single() else {
        return;
    };
    if let Some(children) = children_opt {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let mut groups: Vec<&Army> = army_registry
        .armies
        .values()
        .filter(|g| g.owner == player_cid)
        .collect();
    groups.sort_by_key(|g| g.id.0);

    commands.entity(container_entity).with_children(|list| {
        if groups.is_empty() {
            list.spawn((
                Text::new(tr("military_panel.army_list_empty")),
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
            return;
        }

        for group in groups {
            let is_selected = target_army == Some(group.id);
            let label = trf(
                "military_panel.army_list_row",
                vec![
                    ("name", group.name.clone()),
                    ("count", group.member_division_ids.len().to_string()),
                ],
            );
            let bg = if is_selected {
                ARMY_GROUP_CMD_READY_COLOR
            } else {
                Color::srgba(0.2, 0.2, 0.22, 1.0)
            };
            list.spawn((
                ArmyListRowButton(group.id),
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    ..default()
                },
                BackgroundColor(bg),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(label),
                    TextColor(Color::WHITE),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                ));
            });
        }
    });
}

/// P21-004: 編成コマンドボタン(作成/追加/除外/解散)のクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ命令を発行する
/// (前線命令ボタンと同型のパターン)。
fn handle_army_command_buttons(
    btn_q: Query<(&Interaction, &ArmyCommandButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    selected_division: Res<SelectedDivision>,
    mut army_registry: ResMut<ArmyRegistry>,
    mut selected_army: ResMut<SelectedArmy>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let selected_ids = selected_division.sorted_ids();
        match btn.0 {
            ArmyCommand::Create => execute_army_create(
                &mut army_registry,
                &military_registry,
                &selected_ids,
                player_cid,
                &mut selected_army,
            ),
            ArmyCommand::AddSelection => execute_army_add_selection(
                &mut army_registry,
                &military_registry,
                &selected_ids,
                player_cid,
                *selected_army,
            ),
            ArmyCommand::RemoveSelection => execute_army_remove_selection(
                &mut army_registry,
                &military_registry,
                &selected_ids,
                player_cid,
            ),
            ArmyCommand::Disband => {
                execute_army_disband(&mut army_registry, player_cid, &mut selected_army)
            }
        }
    }
}

/// P21-004: 編成一覧の行(`ArmyListRowButton`)のクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ選択を切り替える。
fn handle_army_list_row_clicks(
    btn_q: Query<(&Interaction, &ArmyListRowButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    army_registry: Res<ArmyRegistry>,
    mut selected_division: ResMut<SelectedDivision>,
    mut selected_army: ResMut<SelectedArmy>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        execute_army_list_row_click(
            &army_registry,
            &mut selected_division,
            &mut selected_army,
            btn.0,
            player_cid,
        );
    }
}

/// P21-005: 前線割当ボタン(設定/解除)の背景色・現在の割当状況テキストを更新する。
/// `update_army_ui`とは別のSystemに切り出す(`ArmyCommandButton`クエリと同一エンティティを
/// 取り合わないための独立コンポーネント設計に合わせ、更新Systemも独立させる)。
#[allow(clippy::too_many_arguments)]
fn update_army_frontline_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    army_registry: Res<ArmyRegistry>,
    selected_army: Res<SelectedArmy>,
    frontline_registry: Res<FrontlineRegistry>,
    war_registry: Res<WarRegistry>,
    country_registry: Res<CountryRegistry>,
    frontline_select_mode: Res<FrontlineSelectMode>,
    loc: crate::localization::Loc,
    mut btn_q: Query<(&ArmyFrontlineCommandButton, &mut BackgroundColor)>,
    mut status_text_q: Query<&mut Text, With<ArmyFrontlineStatusText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let tr = |key: &'static str| t(catalog, locale.0, key);
    let trf =
        |key: &'static str, args: Vec<(&'static str, String)>| tf(catalog, locale.0, key, args);

    let feasibility = evaluate_army_frontline_assign_feasibility(
        selected_army.0,
        player_cid,
        &army_registry,
        &frontline_registry,
        &war_registry,
    );
    let mode_active_for_selection =
        frontline_select_mode.is_active() && frontline_select_mode.army_id == selected_army.0;
    let current_assignment = selected_army
        .0
        .and_then(|id| frontline_registry.frontline_for_army(id));

    for (btn, mut bg) in btn_q.iter_mut() {
        let ready = match btn.0 {
            ArmyFrontlineCommand::Assign => feasibility.is_ready(),
            ArmyFrontlineCommand::Unassign => current_assignment.is_some(),
        };
        *bg = BackgroundColor(
            if mode_active_for_selection && btn.0 == ArmyFrontlineCommand::Assign {
                FRONTLINE_STANCE_ACTIVE_COLOR
            } else if ready {
                ARMY_GROUP_CMD_READY_COLOR
            } else {
                ARMY_GROUP_CMD_DISABLED_COLOR
            },
        );
    }

    let status_line = if mode_active_for_selection {
        tr("military_panel.army_frontline_status_selecting")
    } else if let Some(fl_id) = current_assignment {
        let army_owner = selected_army
            .0
            .and_then(|id| army_registry.armies.get(&id))
            .map(|a| a.owner);
        let enemy_name = frontline_registry
            .frontlines
            .get(&fl_id)
            .zip(army_owner)
            .and_then(|(fl, owner)| {
                let enemy_id = if owner == fl.attacker_country_id {
                    fl.defender_country_id
                } else {
                    fl.attacker_country_id
                };
                country_registry.get(enemy_id).map(|c| c.name.clone())
            })
            .unwrap_or_else(|| tr("common.unknown"));
        trf(
            "military_panel.army_frontline_status_assigned",
            vec![("enemy", enemy_name)],
        )
    } else {
        match feasibility {
            ArmyFrontlineAssignFeasibility::NoArmySelected => {
                tr("military_panel.army_frontline_status_none_selected")
            }
            ArmyFrontlineAssignFeasibility::NoAssignableFrontline => {
                tr("military_panel.army_frontline_status_no_assignable")
            }
            _ => tr("military_panel.army_frontline_status_unassigned"),
        }
    };
    if let Ok(mut text) = status_text_q.single_mut()
        && text.0 != status_line
    {
        text.0 = status_line;
    }
}

/// P21-005: 前線割当ボタン(設定/解除)のクリックを処理する。`Changed<Interaction>`+
/// `Pressed`によりクリック1回につき1回だけ命令を発行する(既存の各コマンドボタンと
/// 同型のパターン)。
#[allow(clippy::too_many_arguments)]
fn handle_army_frontline_command_buttons(
    btn_q: Query<(&Interaction, &ArmyFrontlineCommandButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    army_registry: Res<ArmyRegistry>,
    selected_army: Res<SelectedArmy>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut frontline_select_mode: ResMut<FrontlineSelectMode>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            ArmyFrontlineCommand::Assign => {
                execute_army_frontline_assign_toggle(*selected_army, &mut frontline_select_mode);
            }
            ArmyFrontlineCommand::Unassign => {
                if execute_army_frontline_unassign(
                    &mut frontline_registry,
                    &army_registry,
                    *selected_army,
                    player_cid,
                ) {
                    notif_writer.write(GameNotification {
                        message: t(
                            &catalog,
                            locale.0,
                            "military_panel.army_frontline_unassign_success",
                        ),
                    });
                }
            }
        }
    }
}

/// P21-005: 前線選択モード中の地図クリック結果(`map::frontline_selection`が発行)を
/// 購読し、ローカライズ済みの通知(割当成功/無効な前線)を表示する。実際のクリック
/// 判定・`assign_army`呼び出し自体はmap層(`map::frontline_selection`)が行う
/// (このSystemは通知の構築だけを担当する)。
fn handle_frontline_assignment_attempted(
    mut events: MessageReader<FrontlineAssignmentAttempted>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    for event in events.read() {
        let key = match event {
            FrontlineAssignmentAttempted::Assigned { .. } => {
                "military_panel.army_frontline_assign_success"
            }
            FrontlineAssignmentAttempted::Invalid => "military_panel.army_frontline_invalid_click",
        };
        notif_writer.write(GameNotification {
            message: t(&catalog, locale.0, key),
        });
    }
}

/// P21-005: Armyパネルを閉じたら前線選択モードを解除する
/// (`map::frontline_selection`はui層に依存できないため、この1条件だけはui層が担う)。
fn cancel_frontline_select_mode_on_panel_close(
    state: Res<MilitaryPanelState>,
    mut mode: ResMut<FrontlineSelectMode>,
) {
    if !state.open {
        mode.cancel();
    }
}

/// P21-007: 攻勢線編集モードをトグルする。既にこのArmyで編集中ならキャンセルし、
/// そうでなければ対象Armyの前線割当を確認したうえで、既存の確定済み攻勢線を
/// draftの初期値として編集モードを開始する(再編集時に現在値が表示される仕様)。
fn execute_offensive_line_start_edit_toggle(
    selected_army: SelectedArmy,
    player_cid: CountryId,
    army_registry: &ArmyRegistry,
    frontline_registry: &FrontlineRegistry,
    mode: &mut OffensiveLineEditMode,
) {
    let Some(army_id) = selected_army.0 else {
        return;
    };
    if mode.army_id == Some(army_id) {
        mode.cancel();
        return;
    }
    let Some(army) = army_registry.armies.get(&army_id) else {
        return;
    };
    if army.owner != player_cid {
        return;
    }
    let Some(fl_id) = frontline_registry.frontline_for_army(army_id) else {
        return;
    };
    let initial = frontline_registry
        .get_plan(fl_id, player_cid)
        .and_then(|p| p.offensive_line_region_ids.clone())
        .unwrap_or_default();
    mode.activate(army_id, fl_id, player_cid, initial);
}

/// P21-007: 攻勢線を即座に解除する(確認ダイアログなしの仕様)。成功したかを返す
/// (通知表示用)。編集中であれば編集モードも同時にキャンセルする(解除後の確定済み値と
/// 乖離したdraftを編集し続けさせないため)。
fn execute_offensive_line_clear(
    selected_army: SelectedArmy,
    player_cid: CountryId,
    army_registry: &ArmyRegistry,
    frontline_registry: &mut FrontlineRegistry,
    mode: &mut OffensiveLineEditMode,
) -> bool {
    let Some(army_id) = selected_army.0 else {
        return false;
    };
    let Some(army) = army_registry.armies.get(&army_id) else {
        return false;
    };
    if army.owner != player_cid {
        return false;
    }
    let Some(fl_id) = frontline_registry.frontline_for_army(army_id) else {
        return false;
    };
    frontline_registry.clear_offensive_line(fl_id, player_cid);
    if mode.army_id == Some(army_id) {
        mode.cancel();
    }
    true
}

/// P21-007: 攻勢線(設定/解除/確定/キャンセル)ボタンの背景色・状態テキストを更新する。
/// `update_army_frontline_ui`と同じ理由(SystemParamタプル引数数上限)で独立System化。
#[allow(clippy::too_many_arguments)]
fn update_offensive_line_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    army_registry: Res<ArmyRegistry>,
    selected_army: Res<SelectedArmy>,
    frontline_registry: Res<FrontlineRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mode: Res<OffensiveLineEditMode>,
    loc: crate::localization::Loc,
    mut btn_q: Query<(&OffensiveLineCommandButton, &mut BackgroundColor)>,
    mut status_text_q: Query<&mut Text, With<OffensiveLineStatusText>>,
) {
    let locale = &loc.locale;
    let catalog = &loc.catalog;
    if !state.open && !locale.is_changed() {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    let tr = |key: &'static str| t(catalog, locale.0, key);
    let trf =
        |key: &'static str, args: Vec<(&'static str, String)>| tf(catalog, locale.0, key, args);

    let selected_army_id = selected_army.0;
    let assignment = selected_army_id
        .filter(|&id| {
            army_registry
                .armies
                .get(&id)
                .is_some_and(|a| a.owner == player_cid)
        })
        .and_then(|id| {
            frontline_registry
                .frontline_for_army(id)
                .map(|fl_id| (id, fl_id))
        });

    let mode_active_for_selection = mode.is_active() && mode.army_id == selected_army_id;

    let current_line: Option<Vec<StateId>> = assignment.and_then(|(_, fl_id)| {
        frontline_registry
            .get_plan(fl_id, player_cid)
            .and_then(|p| p.offensive_line_region_ids.clone())
    });

    let start_edit_ready = assignment.is_some();
    let clear_ready = assignment.is_some() && current_line.is_some();
    let confirm_ready = mode_active_for_selection && !mode.draft.is_empty();
    let cancel_ready = mode_active_for_selection;

    for (btn, mut bg) in btn_q.iter_mut() {
        let ready = match btn.0 {
            OffensiveLineCommand::StartEdit => start_edit_ready,
            OffensiveLineCommand::Clear => clear_ready,
            OffensiveLineCommand::Confirm => confirm_ready,
            OffensiveLineCommand::Cancel => cancel_ready,
        };
        *bg = BackgroundColor(
            if mode_active_for_selection && btn.0 == OffensiveLineCommand::StartEdit {
                FRONTLINE_STANCE_ACTIVE_COLOR
            } else if ready {
                ARMY_GROUP_CMD_READY_COLOR
            } else {
                ARMY_GROUP_CMD_DISABLED_COLOR
            },
        );
    }

    let status_line = if selected_army_id.is_none() {
        tr("military_panel.offensive_line_status_none_selected")
    } else if assignment.is_none() {
        tr("military_panel.offensive_line_status_not_assigned")
    } else if mode_active_for_selection {
        trf(
            "military_panel.offensive_line_status_editing",
            vec![("count", mode.draft.len().to_string())],
        )
    } else {
        match &current_line {
            // P21-008: 攻勢線が設定済みなら、既存軍事パネル構造のまま(新しい行/ボタンを
            // 追加せず)このステータス行だけを5状態(未設定/準備中/実行中/到達済み/
            // 到達可能な目標なし)へ拡張する。`compute_offensive_line_progress`は
            // `plan.offensive_line_region_ids`自体もNotSet判定に使うため、
            // `current_line`がNoneの場合と結果が食い違うことはない。
            Some(_) => {
                let fl_id = assignment.map(|(_, fl_id)| fl_id);
                let progress = fl_id.and_then(|fl_id| {
                    let plan = frontline_registry.get_plan(fl_id, player_cid)?;
                    let frontline = frontline_registry.frontlines.get(&fl_id)?;
                    Some(compute_offensive_line_progress(
                        plan,
                        frontline,
                        &state_registry,
                        &military_registry,
                        &army_registry,
                        &frontline_registry,
                    ))
                });
                match progress {
                    Some(OffensiveLineProgress::NotSet) | None => {
                        tr("military_panel.offensive_line_status_unset")
                    }
                    Some(OffensiveLineProgress::Preparing) => {
                        tr("military_panel.offensive_line_status_preparing")
                    }
                    Some(OffensiveLineProgress::InProgress) => trf(
                        "military_panel.offensive_line_status_in_progress",
                        vec![(
                            "count",
                            uncaptured_offensive_line_regions(
                                current_line.as_deref().unwrap_or(&[]),
                                &state_registry,
                                player_cid,
                            )
                            .len()
                            .to_string(),
                        )],
                    ),
                    Some(OffensiveLineProgress::Reached) => {
                        tr("military_panel.offensive_line_status_reached")
                    }
                    Some(OffensiveLineProgress::NoReachableTargets) => {
                        tr("military_panel.offensive_line_status_no_reachable_targets")
                    }
                }
            }
            None => tr("military_panel.offensive_line_status_unset"),
        }
    };
    if let Ok(mut text) = status_text_q.single_mut()
        && text.0 != status_line
    {
        text.0 = status_line;
    }
}

/// P21-007: 攻勢線ボタン(設定/解除/確定/キャンセル)のクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ命令を発行する。
#[allow(clippy::too_many_arguments)]
fn handle_offensive_line_command_buttons(
    btn_q: Query<(&Interaction, &OffensiveLineCommandButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    army_registry: Res<ArmyRegistry>,
    selected_army: Res<SelectedArmy>,
    state_registry: Res<StateRegistry>,
    war_registry: Res<WarRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut mode: ResMut<OffensiveLineEditMode>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player_cid) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            OffensiveLineCommand::StartEdit => {
                execute_offensive_line_start_edit_toggle(
                    *selected_army,
                    player_cid,
                    &army_registry,
                    &frontline_registry,
                    &mut mode,
                );
            }
            OffensiveLineCommand::Clear => {
                if execute_offensive_line_clear(
                    *selected_army,
                    player_cid,
                    &army_registry,
                    &mut frontline_registry,
                    &mut mode,
                ) {
                    notif_writer.write(GameNotification {
                        message: t(
                            &catalog,
                            locale.0,
                            "military_panel.offensive_line_clear_success",
                        ),
                    });
                }
            }
            OffensiveLineCommand::Confirm => {
                let (Some(fl_id), Some(country_id)) = (mode.frontline_id, mode.country_id) else {
                    continue;
                };
                let draft = mode.draft.clone();
                match frontline_registry.set_offensive_line(
                    fl_id,
                    country_id,
                    &draft,
                    &state_registry,
                    &war_registry,
                ) {
                    Ok(()) => {
                        mode.cancel();
                        notif_writer.write(GameNotification {
                            message: t(
                                &catalog,
                                locale.0,
                                "military_panel.offensive_line_set_success",
                            ),
                        });
                    }
                    Err(_) => {
                        notif_writer.write(GameNotification {
                            message: t(
                                &catalog,
                                locale.0,
                                "military_panel.offensive_line_confirm_invalid",
                            ),
                        });
                    }
                }
            }
            OffensiveLineCommand::Cancel => {
                mode.cancel();
            }
        }
    }
}

/// P21-007: 軍事パネルを閉じたら攻勢線編集モードを解除する
/// (`cancel_frontline_select_mode_on_panel_close`と同じ理由: `map::offensive_line_selection`は
/// ui層に依存できないため、この1条件だけはui層が担う)。
fn cancel_offensive_line_edit_on_panel_close(
    state: Res<MilitaryPanelState>,
    mut mode: ResMut<OffensiveLineEditMode>,
) {
    if !state.open {
        mode.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::StateId;
    use crate::country::CountryData;
    use crate::localization::MISSING_KEY_MARKER_PREFIX;
    use crate::military::data::{Division, DivisionDefinition, DivisionSize, DivisionType};
    use crate::state::data::StateData;
    use crate::war::data::{War, WarStatus};
    use crate::war::frontline::{Frontline, FrontlinePlan};

    fn test_division() -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(1),
            name: "Test Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 15.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 5.0,
        }
    }

    fn owned_state(id: usize, owner: crate::common::CountryId) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: owner,
            ..Default::default()
        }
    }

    /// `handle_recruit_buttons`を`Changed<Interaction>`込みで実行できる最小限のAppを構築する。
    /// レンダリング・アセット・ゲームプラグイン一式には依存しない(`MinimalPlugins`のみ)。
    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<GameNotification>();

        let mut military_registry = MilitaryRegistry::default();
        military_registry
            .definitions
            .insert(DivisionDefinitionId(1), test_division());
        app.insert_resource(military_registry);

        let country = CountryData {
            id: crate::common::CountryId(1),
            available_manpower: 20_000,
            treasury: 500.0,
            ..Default::default()
        };
        let mut country_registry = CountryRegistry::default();
        country_registry.countries.push(country);
        app.insert_resource(country_registry);

        app.insert_resource(StateRegistry::build(vec![owned_state(
            5,
            crate::common::CountryId(1),
        )]));
        app.insert_resource(PlayerCountry(Some(crate::common::CountryId(1))));
        app.insert_resource(SelectedState(Some(StateId(5))));
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));

        app
    }

    fn press_recruit_button(app: &mut App, division_id: DivisionDefinitionId) {
        app.world_mut()
            .spawn((RecruitButton(division_id), Interaction::Pressed));
    }

    fn player_country_data(app: &App) -> &CountryData {
        app.world()
            .resource::<CountryRegistry>()
            .get(crate::common::CountryId(1))
            .expect("player country must exist")
    }

    #[test]
    fn handle_recruit_buttons_success_queues_recruitment_and_deducts_cost() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(1));

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 1);
        assert_eq!(country.available_manpower, 10_000);
        assert_eq!(country.treasury, 400.0);
        assert_eq!(country.mobilized_manpower, 10_000);
    }

    #[test]
    fn handle_recruit_buttons_insufficient_funds_does_not_mutate_state() {
        let mut app = build_test_app();
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(crate::common::CountryId(1))
            .unwrap()
            .treasury = 1.0;
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(1));

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 20_000);
        assert_eq!(country.treasury, 1.0);
    }

    #[test]
    fn handle_recruit_buttons_insufficient_manpower_does_not_mutate_state() {
        let mut app = build_test_app();
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(crate::common::CountryId(1))
            .unwrap()
            .available_manpower = 100;
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(1));

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 100);
        assert_eq!(country.treasury, 500.0);
    }

    #[test]
    fn handle_recruit_buttons_foreign_state_does_not_mutate_state() {
        let mut app = build_test_app();
        *app.world_mut().resource_mut::<StateRegistry>() =
            StateRegistry::build(vec![owned_state(5, crate::common::CountryId(2))]);
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(1));

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 20_000);
        assert_eq!(country.treasury, 500.0);
    }

    #[test]
    fn handle_recruit_buttons_no_selection_does_not_mutate_state() {
        let mut app = build_test_app();
        app.insert_resource(SelectedState(None));
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(1));

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 20_000);
        assert_eq!(country.treasury, 500.0);
    }

    #[test]
    fn handle_recruit_buttons_unknown_definition_does_not_mutate_state() {
        let mut app = build_test_app();
        app.add_systems(Update, handle_recruit_buttons);
        press_recruit_button(&mut app, DivisionDefinitionId(999)); // 未定義の部隊ID

        app.update();

        let country = player_country_data(&app);
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 20_000);
        assert_eq!(country.treasury, 500.0);
    }

    /// UI接続確認: RecruitButtonが実際に軍事パネルのUIツリーへspawnされていること
    /// (P21-001以前は定義のみで到達不能なコードだった)。
    #[test]
    fn recruit_button_is_spawned_in_military_panel_ui_tree() {
        let mut app = build_test_app();
        app.add_systems(Startup, setup_military_panel);
        app.update();

        let count = app
            .world_mut()
            .query::<&RecruitButton>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 1,
            "RecruitButton must be present in the spawned military panel UI tree"
        );
    }

    // ── P21-002: 前線命令ボタンのテスト ──────────────────────────

    fn make_frontline_test_division(
        id: usize,
        owner: crate::common::CountryId,
        state: StateId,
    ) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    /// 自国(CountryId(1))・敵国(CountryId(2))それぞれの陸軍を1個ずつ配置し、
    /// 両国のFrontlinePlanを持つ前線をあらかじめ生成したテスト環境を構築する。
    /// 敵国の陸軍は敵国自身のプランへあらかじめ割当済みにしておく(6-1回帰テスト用)。
    /// 返り値は (App, 自国division_id, 敵国division_id, frontline_id)。
    fn build_frontline_command_test_app() -> (App, DivisionId, DivisionId, FrontlineId) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let player_cid = CountryId(1);
        let enemy_cid = CountryId(2);

        let mut military_registry = MilitaryRegistry::default();
        let own_division_id =
            military_registry.add_division(make_frontline_test_division(0, player_cid, StateId(1)));
        let enemy_division_id =
            military_registry.add_division(make_frontline_test_division(1, enemy_cid, StateId(2)));
        app.insert_resource(military_registry);

        let war_id = crate::common::WarId(0);
        let mut war_registry = WarRegistry::default();
        war_registry.wars.insert(
            war_id,
            War {
                id: war_id,
                name: "Test War".to_string(),
                attackers: [player_cid].into_iter().collect(),
                defenders: [enemy_cid].into_iter().collect(),
                war_goals: Vec::new(),
                start_date: "1800/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: Default::default(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: Default::default(),
            },
        );
        app.insert_resource(war_registry);

        let mut frontline_registry = FrontlineRegistry::default();
        let fl_id = frontline_registry.generate_id();
        frontline_registry.frontlines.insert(
            fl_id,
            Frontline {
                frontline_id: fl_id,
                war_id,
                attacker_country_id: player_cid,
                defender_country_id: enemy_cid,
                attacker_front_regions: Vec::new(),
                defender_front_regions: Vec::new(),
                border_region_pairs: Vec::new(),
            },
        );
        frontline_registry
            .plans
            .insert((fl_id, player_cid), FrontlinePlan::new(fl_id, player_cid));
        frontline_registry
            .plans
            .insert((fl_id, enemy_cid), FrontlinePlan::new(fl_id, enemy_cid));
        frontline_registry
            .plans
            .get_mut(&(fl_id, enemy_cid))
            .unwrap()
            .assigned_division_ids
            .push(enemy_division_id);
        frontline_registry
            .division_frontline_map
            .insert(enemy_division_id, fl_id);
        app.insert_resource(frontline_registry);

        app.insert_resource(PlayerCountry(Some(player_cid)));
        app.insert_resource(SelectedDivision {
            division_ids: [own_division_id].into_iter().collect(),
        });

        (app, own_division_id, enemy_division_id, fl_id)
    }

    fn press_frontline_command_button(app: &mut App, cmd: FrontlineCommand) {
        app.world_mut()
            .spawn((FrontlineCommandButton(cmd), Interaction::Pressed));
    }

    /// UI接続確認: 前線命令ボタン6個(割当/解除/全解除/停止/防御/攻勢)が
    /// 実際に軍事パネルのUIツリーへspawnされていること。
    #[test]
    fn frontline_command_buttons_are_spawned_in_military_panel_ui_tree() {
        let mut app = build_test_app();
        app.add_systems(Startup, setup_military_panel);
        app.update();

        let count = app
            .world_mut()
            .query::<&FrontlineCommandButton>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 6,
            "all 6 frontline command buttons must be present in the spawned military panel UI tree"
        );

        let info_text_count = app
            .world_mut()
            .query::<&FrontlineCommandInfoText>()
            .iter(app.world())
            .count();
        assert_eq!(info_text_count, 1);
    }

    #[test]
    fn handle_frontline_command_buttons_assign_success() {
        let (mut app, own_division_id, _enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Assign);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert_eq!(
            frontline_registry
                .division_frontline_map
                .get(&own_division_id),
            Some(&fl_id)
        );
    }

    /// P21-002最重要回帰テスト: 選択中陸軍が敵国のものであっても
    /// (`SelectedDivision`は所有者を問わず選択され得るため)、解除ボタンを押しても
    /// 敵国の前線割当は一切変化しない(6-1のバグ修正確認)。
    #[test]
    fn handle_frontline_command_buttons_unassign_rejects_foreign_division() {
        let (mut app, _own_division_id, enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        // 選択中陸軍を敵国のものに差し替える(左クリックで選択され得る状態を再現)
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .select_only(enemy_division_id);
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Unassign);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert_eq!(
            frontline_registry
                .division_frontline_map
                .get(&enemy_division_id),
            Some(&fl_id),
            "unassign button must not remove another country's frontline assignment"
        );
        let enemy_plan = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();
        assert!(
            enemy_plan
                .assigned_division_ids
                .contains(&enemy_division_id)
        );
    }

    #[test]
    fn handle_frontline_command_buttons_unassign_all_only_affects_own_plan() {
        let (mut app, own_division_id, enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        {
            let mut frontline_registry = app.world_mut().resource_mut::<FrontlineRegistry>();
            frontline_registry
                .plans
                .get_mut(&(fl_id, CountryId(1)))
                .unwrap()
                .assigned_division_ids
                .push(own_division_id);
            frontline_registry
                .division_frontline_map
                .insert(own_division_id, fl_id);
        }
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::UnassignAll);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert!(
            !frontline_registry
                .division_frontline_map
                .contains_key(&own_division_id),
            "own division must be unassigned"
        );
        assert_eq!(
            frontline_registry
                .division_frontline_map
                .get(&enemy_division_id),
            Some(&fl_id),
            "enemy country's plan must not be affected by the player's unassign-all"
        );
    }

    #[test]
    fn handle_frontline_command_buttons_set_stance() {
        let (mut app, _own_division_id, _enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(
            &mut app,
            FrontlineCommand::SetStance(FrontlineStance::Offensive),
        );

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        let own_plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(own_plan.stance, FrontlineStance::Offensive);
        let enemy_plan = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();
        assert_eq!(
            enemy_plan.stance,
            FrontlineStance::Stopped,
            "setting the player's stance must not affect the enemy country's plan"
        );
    }

    #[test]
    fn handle_frontline_command_buttons_no_active_war_does_nothing() {
        let (mut app, own_division_id, _enemy_division_id, _fl_id) =
            build_frontline_command_test_app();
        app.insert_resource(WarRegistry::default());
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Assign);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert!(
            !frontline_registry
                .division_frontline_map
                .contains_key(&own_division_id),
            "no active war means no frontline command should have any effect"
        );
    }

    #[test]
    fn handle_frontline_command_buttons_fires_once_per_press() {
        let (mut app, own_division_id, _enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Assign);

        app.update();
        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .division_frontline_map
                .get(&own_division_id),
            Some(&fl_id)
        );

        // 割当後に手動で解除し、Interactionを変化させないまま再度updateしても
        // 再度Assignは実行されない(Changed<Interaction>により1回のPressにつき1回だけ発行)
        app.world_mut()
            .resource_mut::<FrontlineRegistry>()
            .unassign_all_divisions_for_plan(fl_id, CountryId(1));
        app.update();

        assert!(
            !app.world()
                .resource::<FrontlineRegistry>()
                .division_frontline_map
                .contains_key(&own_division_id),
            "without a new Interaction change, the button must not fire again"
        );
    }

    /// 自国(CountryId(1))2陸軍・敵国(CountryId(2))1陸軍を配置した編成(Army)
    /// テスト環境を構築する。返り値は(App, own_division_1, own_division_2, enemy_division)。
    fn build_army_command_test_app() -> (App, DivisionId, DivisionId, DivisionId) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let player_cid = CountryId(1);
        let enemy_cid = CountryId(2);

        let mut military_registry = MilitaryRegistry::default();
        let own_division_1 =
            military_registry.add_division(make_frontline_test_division(0, player_cid, StateId(1)));
        let own_division_2 =
            military_registry.add_division(make_frontline_test_division(1, player_cid, StateId(1)));
        let enemy_division =
            military_registry.add_division(make_frontline_test_division(2, enemy_cid, StateId(2)));
        app.insert_resource(military_registry);

        app.insert_resource(ArmyRegistry::default());
        app.insert_resource(SelectedArmy::default());
        app.insert_resource(PlayerCountry(Some(player_cid)));
        app.insert_resource(SelectedDivision {
            division_ids: [own_division_1].into_iter().collect(),
        });
        app.init_resource::<crate::map::frontline_selection::FrontlineSelectMode>();

        (app, own_division_1, own_division_2, enemy_division)
    }

    fn press_army_command_button(app: &mut App, cmd: ArmyCommand) {
        app.world_mut()
            .spawn((ArmyCommandButton(cmd), Interaction::Pressed));
    }

    fn press_army_list_row(app: &mut App, id: crate::common::ArmyId) {
        app.world_mut()
            .spawn((ArmyListRowButton(id), Interaction::Pressed));
    }

    /// UI接続確認: 編成コマンドボタン4個(作成/追加/除外/解散)と、
    /// 選択中編成表示・編成一覧コンテナが軍事パネルUIツリーへspawnされていること。
    #[test]
    fn army_command_buttons_are_spawned_in_military_panel_ui_tree() {
        let mut app = build_test_app();
        app.add_systems(Startup, setup_military_panel);
        app.update();

        let count = app
            .world_mut()
            .query::<&ArmyCommandButton>()
            .iter(app.world())
            .count();
        assert_eq!(
            count, 4,
            "all 4 army command buttons must be present in the spawned military panel UI tree"
        );

        assert_eq!(
            app.world_mut()
                .query::<&ArmyStatusText>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world_mut()
                .query::<&ArmyListContainer>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn handle_army_command_buttons_create_success() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1, own_division_2].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Create);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(registry.armies.len(), 1);
        let group = registry.armies.values().next().unwrap();
        assert_eq!(
            group.member_division_ids,
            vec![own_division_1, own_division_2]
        );
        assert_eq!(group.owner, CountryId(1));
        let group_id = group.id;

        // P21-004: 作成後、そのArmyが選択状態になる
        assert_eq!(app.world().resource::<SelectedArmy>().0, Some(group_id));
    }

    /// P21-003監査で発見した「選択が所有者を問わない」不具合が編成側に波及しないことの
    /// 回帰テスト: 敵国陸軍が選択に混ざっていても、編成には自国陸軍だけが入る。
    #[test]
    fn handle_army_command_buttons_create_ignores_foreign_division() {
        let (mut app, own_division_1, _own_division_2, enemy_division) =
            build_army_command_test_app();
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1, enemy_division].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Create);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        let group = registry.armies.values().next().unwrap();
        assert_eq!(group.member_division_ids, vec![own_division_1]);
        assert_eq!(registry.army_for_division(enemy_division), None);
    }

    /// テスト内で`ArmyRegistry`(可変)と`MilitaryRegistry`(不変)を同時に必要とする
    /// `create_army`呼び出し用のヘルパー。`app.world_mut()`と`app.world()`を同一式内で
    /// 借用できないため、`resource_scope`で安全に両方へアクセスする。
    fn create_test_group(
        app: &mut App,
        owner: CountryId,
        member_ids: &[DivisionId],
    ) -> crate::common::ArmyId {
        app.world_mut()
            .resource_scope(|world, mut registry: Mut<ArmyRegistry>| {
                registry
                    .create_army(owner, member_ids, world.resource::<MilitaryRegistry>())
                    .unwrap()
            })
    }

    #[test]
    fn handle_army_command_buttons_add_selection_adds_ungrouped_member() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1]);
        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(group_id);

        // 既存グループ所属の師団1 + 未所属の師団2 を選択してAdd
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1, own_division_2].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::AddSelection);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(
            registry.armies[&group_id].member_division_ids,
            vec![own_division_1, own_division_2]
        );
        assert_eq!(registry.army_for_division(own_division_2), Some(group_id));
    }

    #[test]
    fn handle_army_command_buttons_remove_selection() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::RemoveSelection);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(registry.army_for_division(own_division_1), None);
        assert_eq!(
            registry.armies[&group_id].member_division_ids,
            vec![own_division_2]
        );
    }

    /// P21-004: 編成一覧の行をクリックすると、選択中編成が切り替わり、
    /// その所属する生存中の全師団がDivision選択へ反映される(一括選択の入口)。
    #[test]
    fn clicking_army_list_row_selects_army_and_expands_division_selection() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        // 師団1だけを選択した状態から編成一覧の行をクリック
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_list_row_clicks);
        press_army_list_row(&mut app, group_id);

        app.update();

        assert_eq!(app.world().resource::<SelectedArmy>().0, Some(group_id));
        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 2);
        assert!(selected.is_selected(own_division_1));
        assert!(selected.is_selected(own_division_2));
    }

    /// 他国の編成の行をクリックしても、選択中編成・Division選択のどちらも変化しない。
    #[test]
    fn clicking_foreign_army_list_row_is_ignored() {
        let (mut app, own_division_1, _own_division_2, enemy_division) =
            build_army_command_test_app();
        let enemy_group_id = create_test_group(&mut app, CountryId(2), &[enemy_division]);

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_list_row_clicks);
        press_army_list_row(&mut app, enemy_group_id);

        app.update();

        assert_eq!(app.world().resource::<SelectedArmy>().0, None);
        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 1);
        assert!(selected.is_selected(own_division_1));
    }

    /// P21-004 spec #8: 軍をクリックして選択した後、既存の一括移動命令
    /// (`map::division_selection::handle_movement_order`)がそのまま機能することを検証する。
    /// Army選択がDivision選択を置き換えるのではなく「一括選択の入口」として実装されている
    /// ことの直接的な証明(移動処理そのものは一切変更していない)。
    #[test]
    fn army_selection_feeds_into_existing_bulk_movement_order() {
        use crate::map::camera::GameCamera;
        use crate::map::division_selection::handle_movement_order;
        use crate::military::battle::BattleRegistry;
        use bevy::window::WindowResolution;

        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(1)],
            world_position: [300.0, 0.0],
            size: [100.0, 100.0],
            ..Default::default()
        };
        app.insert_resource(StateRegistry::build(vec![s1, s2]));
        app.insert_resource(WarRegistry::default());
        app.insert_resource(BattleRegistry::default());

        // 軍一覧の行をクリックして選択(所属する2師団がDivision選択へ反映される)
        app.add_systems(Update, handle_army_list_row_clicks);
        press_army_list_row(&mut app, group_id);
        app.update();
        assert_eq!(app.world().resource::<SelectedDivision>().len(), 2);

        // クリックしたUIボタンをその場に残したままだと、`handle_movement_order`自身の
        // 「UI要素がPressed/Hovered中はマップ操作を無視する」ガードに引っかかって
        // 移動命令が発行されなくなる。実際のプレイでは次フレームでInteractionが
        // 遷移するのと同じ状況を再現するため、明示的にボタンを片付ける。
        let stale_buttons: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, With<ArmyListRowButton>>()
            .iter(app.world())
            .collect();
        for entity in stale_buttons {
            app.world_mut().entity_mut(entity).despawn();
        }

        // 既存の一括移動命令(右クリック)を、システムそのものを一切変更せずに発行する
        app.add_systems(Update, handle_movement_order);
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Right);
        app.insert_resource(mouse);
        app.world_mut()
            .spawn((Camera2d, GameCamera, Transform::default()));
        let mut window = Window {
            resolution: WindowResolution::new(800, 600),
            ..default()
        };
        // StateId(2)(300, 0)に対応する画面座標(カメラ原点・scale=1前提)
        let window_size = Vec2::new(800.0, 600.0);
        let target_world = Vec2::new(300.0, 0.0);
        let half_size = window_size * 0.5;
        let ndc = target_world / half_size;
        let raw_ndc = Vec2::new(ndc.x, -ndc.y);
        let screen = (raw_ndc + Vec2::ONE) * 0.5 * window_size;
        window.set_cursor_position(Some(screen));
        app.world_mut().spawn(window);

        app.update();

        let military_registry = app.world().resource::<MilitaryRegistry>();
        assert_eq!(
            military_registry.divisions[&own_division_1].destination,
            Some(StateId(2)),
            "軍選択で一括選択された師団1体目に移動命令が届くはず"
        );
        assert_eq!(
            military_registry.divisions[&own_division_2].destination,
            Some(StateId(2)),
            "軍選択で一括選択された師団2体目にも移動命令が届くはず"
        );
    }

    /// P21-004 spec #9: 軍をクリックして選択した後、選択中師団に対して機能する
    /// 既存の前線コマンド(ここでは選択依存の`Assign`)が引き続き機能することを検証する。
    /// なお「一括停止」に対応する`SetStance(Stopped)`はプラン全体に効く設定であり
    /// 選択集合に依存しないため(この挙動は今回変更していない)、選択依存の一括操作の
    /// 代表としてAssignを検証する。
    #[test]
    fn army_selection_feeds_into_existing_frontline_division_command() {
        let (mut app, own_division_id, _enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        app.insert_resource(ArmyRegistry::default());
        app.insert_resource(SelectedArmy::default());
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_id]);

        app.add_systems(
            Update,
            (
                handle_army_list_row_clicks,
                handle_frontline_command_buttons,
            )
                .chain(),
        );
        press_army_list_row(&mut app, group_id);
        press_frontline_command_button(&mut app, FrontlineCommand::Assign);

        app.update();

        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .division_frontline_map
                .get(&own_division_id),
            Some(&fl_id),
            "軍クリックで選択された師団に、既存の前線割当コマンドが機能するはず"
        );
    }

    #[test]
    fn handle_army_command_buttons_disband_returns_members_to_unassigned() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);
        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(group_id);

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Disband);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert!(!registry.armies.contains_key(&group_id));
        assert_eq!(registry.army_for_division(own_division_1), None);
        assert_eq!(registry.army_for_division(own_division_2), None);
        // P21-004: 解散したArmyの選択状態は解除される
        assert_eq!(app.world().resource::<SelectedArmy>().0, None);
    }

    #[test]
    fn handle_army_command_buttons_fires_once_per_press() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1, own_division_2].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Create);

        app.update();
        assert_eq!(app.world().resource::<ArmyRegistry>().armies.len(), 1);

        // 作成後、Interactionを変化させないまま再度updateしても2件目は作られない
        app.update();
        assert_eq!(
            app.world().resource::<ArmyRegistry>().armies.len(),
            1,
            "without a new Interaction change, the button must not fire again"
        );
    }

    fn press_stop_movement_button(app: &mut App) {
        app.world_mut()
            .spawn((StopMovementButton, Interaction::Pressed));
    }

    fn set_moving_with_order(app: &mut App, id: DivisionId, target: StateId) {
        let mut mil = app.world_mut().resource_mut::<MilitaryRegistry>();
        let d = mil.divisions.get_mut(&id).unwrap();
        d.status = DivisionStatus::Moving;
        d.destination = Some(target);
        d.current_path = vec![target];
        d.target_state = Some(target);
        d.movement_progress = 0.5;
    }

    /// P21-004A spec #2: 複数の選択Divisionを一括で移動停止できる。
    #[test]
    fn handle_stop_movement_button_stops_multiple_selected_divisions() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        set_moving_with_order(&mut app, own_division_1, StateId(9));
        set_moving_with_order(&mut app, own_division_2, StateId(9));

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1, own_division_2].into_iter().collect();
        app.add_systems(Update, handle_stop_movement_button);
        press_stop_movement_button(&mut app);

        app.update();

        let mil = app.world().resource::<MilitaryRegistry>();
        for id in [own_division_1, own_division_2] {
            let d = &mil.divisions[&id];
            assert_eq!(d.status, DivisionStatus::Idle);
            assert_eq!(d.destination, None);
            assert!(d.current_path.is_empty());
            assert_eq!(d.target_state, None);
            assert_eq!(d.movement_progress, 0.0);
        }
    }

    /// P21-004A spec #5: 選択されていないDivisionの移動命令は変化しない。
    #[test]
    fn handle_stop_movement_button_leaves_unselected_division_unaffected() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        set_moving_with_order(&mut app, own_division_1, StateId(9));
        set_moving_with_order(&mut app, own_division_2, StateId(9));

        // own_division_1だけを選択する(own_division_2は選択しない)
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_stop_movement_button);
        press_stop_movement_button(&mut app);

        app.update();

        let mil = app.world().resource::<MilitaryRegistry>();
        assert_eq!(mil.divisions[&own_division_1].status, DivisionStatus::Idle);
        let d2 = &mil.divisions[&own_division_2];
        assert_eq!(
            d2.status,
            DivisionStatus::Moving,
            "選択されていない師団の移動命令が変化してはならない"
        );
        assert_eq!(d2.destination, Some(StateId(9)));
    }

    /// P21-004A spec #6: 敵国Divisionが何らかの理由で選択集合に混ざっていても、
    /// (UIとは無関係に)`stop_division_movement`自身の所有者検証により停止されない。
    #[test]
    fn handle_stop_movement_button_cannot_stop_enemy_division() {
        let (mut app, _own_division_1, _own_division_2, enemy_division) =
            build_army_command_test_app();
        set_moving_with_order(&mut app, enemy_division, StateId(9));

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [enemy_division].into_iter().collect();
        app.add_systems(Update, handle_stop_movement_button);
        press_stop_movement_button(&mut app);

        app.update();

        let mil = app.world().resource::<MilitaryRegistry>();
        let d = &mil.divisions[&enemy_division];
        assert_eq!(
            d.status,
            DivisionStatus::Moving,
            "敵国師団は停止できてはならない"
        );
        assert_eq!(d.destination, Some(StateId(9)));
    }

    /// P21-004A spec #3/#4: 軍を選択した直後にも同じ「移動停止」操作が機能し、
    /// 所属する全師団(Army非所属師団と同じ経路)を一括で停止できる。
    #[test]
    fn army_selection_then_stop_movement_stops_all_members() {
        let (mut app, own_division_1, own_division_2, _enemy_division) =
            build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);
        set_moving_with_order(&mut app, own_division_1, StateId(9));
        set_moving_with_order(&mut app, own_division_2, StateId(9));

        app.add_systems(
            Update,
            (handle_army_list_row_clicks, handle_stop_movement_button).chain(),
        );
        press_army_list_row(&mut app, group_id);
        press_stop_movement_button(&mut app);

        app.update();

        let mil = app.world().resource::<MilitaryRegistry>();
        for id in [own_division_1, own_division_2] {
            let d = &mil.divisions[&id];
            assert_eq!(d.status, DivisionStatus::Idle);
            assert_eq!(d.destination, None);
        }
        // Army所属自体は変化しない(spec #7)
        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(registry.army_for_division(own_division_1), Some(group_id));
        assert_eq!(registry.army_for_division(own_division_2), Some(group_id));
    }

    /// P21-004A spec #10: 「移動停止」ボタンは前線スタンス(`FrontlineStance`、既存の
    /// 「停止」ボタンが操作する国家プラン全体設定)には一切触れない。
    #[test]
    fn handle_stop_movement_button_does_not_change_frontline_stance() {
        let (mut app, own_division_id, _enemy_division_id, fl_id) =
            build_frontline_command_test_app();
        {
            let mut fl_reg = app.world_mut().resource_mut::<FrontlineRegistry>();
            fl_reg.get_plan_mut(fl_id, CountryId(1)).unwrap().stance = FrontlineStance::Offensive;
        }
        set_moving_with_order(&mut app, own_division_id, StateId(9));

        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .division_ids = [own_division_id].into_iter().collect();
        app.add_systems(Update, handle_stop_movement_button);
        press_stop_movement_button(&mut app);

        app.update();

        // 師団の移動命令は解除される
        let mil = app.world().resource::<MilitaryRegistry>();
        assert_eq!(mil.divisions[&own_division_id].status, DivisionStatus::Idle);

        // 前線スタンスは変化しない(既存の「停止」ボタンとは無関係)
        let fl_reg = app.world().resource::<FrontlineRegistry>();
        assert_eq!(
            fl_reg.get_plan(fl_id, CountryId(1)).unwrap().stance,
            FrontlineStance::Offensive,
            "移動停止は前線スタンスに影響してはならない"
        );
    }

    // ─── P21-005: Army↔Frontline割当UI ─────────────────────────────────────

    /// C1(自国) vs C2(敵国)のアクティブな戦争・Frontline・C1所有Army1体を用意した
    /// 最小限のAppを構築する。`FrontlineSelectMode`/`GameNotification`/ロケールも
    /// 併せて用意し、`update_army_frontline_ui`/`handle_army_frontline_command_buttons`
    /// 単体で動作確認できるようにする。
    fn build_army_frontline_test_app() -> (App, crate::common::ArmyId, FrontlineId) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<GameNotification>();
        app.add_message::<FrontlineAssignmentAttempted>();

        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            ..Default::default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1)],
            ..Default::default()
        };
        app.insert_resource(StateRegistry::build(vec![s1, s2]));

        let mut country_registry = CountryRegistry::default();
        country_registry.countries.push(CountryData {
            id: CountryId(1),
            name: "Arcadia".to_string(),
            ..Default::default()
        });
        country_registry.countries.push(CountryData {
            id: CountryId(2),
            name: "Dwarf".to_string(),
            ..Default::default()
        });
        app.insert_resource(country_registry);

        let mut military_registry = MilitaryRegistry::default();
        let division = Division {
            id: DivisionId(1),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        military_registry.divisions.insert(division.id, division);
        app.insert_resource(military_registry);

        let mut war_registry = WarRegistry::default();
        let war = War {
            id: crate::common::WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: Default::default(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: Default::default(),
        };
        war_registry.wars.insert(war.id, war);
        app.insert_resource(war_registry);

        let mut frontline_registry = FrontlineRegistry::default();
        {
            let war_registry = app.world().resource::<WarRegistry>();
            let state_registry = app.world().resource::<StateRegistry>();
            let military_registry = app.world().resource::<MilitaryRegistry>();
            crate::war::frontline::update_all_frontlines(
                war_registry,
                state_registry,
                military_registry,
                &mut frontline_registry,
            );
        }
        let fl_id = frontline_registry
            .get_frontline_for_war(crate::common::WarId(0))
            .unwrap()
            .frontline_id;
        app.insert_resource(frontline_registry);

        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(
                CountryId(1),
                &[DivisionId(1)],
                app.world().resource::<MilitaryRegistry>(),
            )
            .unwrap();
        app.insert_resource(army_registry);

        app.insert_resource(PlayerCountry(Some(CountryId(1))));
        app.insert_resource(SelectedArmy(Some(army_id)));
        app.insert_resource(SelectedDivision::default());
        app.insert_resource(FrontlineSelectMode::default());
        app.insert_resource(OffensiveLineEditMode::default());
        app.insert_resource(MilitaryPanelState { open: true });
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));

        (app, army_id, fl_id)
    }

    fn press_army_frontline_button(app: &mut App, cmd: ArmyFrontlineCommand) {
        app.world_mut()
            .spawn((ArmyFrontlineCommandButton(cmd), Interaction::Pressed));
    }

    /// 要求テスト項目23: 選択中の自国Armyがある場合、前線設定/解除ボタンがUIツリーへ
    /// spawnされている。
    #[test]
    fn army_frontline_buttons_are_spawned_in_military_panel_ui_tree() {
        let mut app = build_test_app();
        app.add_systems(Startup, setup_military_panel);
        app.update();

        let world = app.world_mut();
        let assign_count = world
            .query::<&ArmyFrontlineCommandButton>()
            .iter(world)
            .filter(|b| b.0 == ArmyFrontlineCommand::Assign)
            .count();
        let unassign_count = world
            .query::<&ArmyFrontlineCommandButton>()
            .iter(world)
            .filter(|b| b.0 == ArmyFrontlineCommand::Unassign)
            .count();
        assert_eq!(assign_count, 1);
        assert_eq!(unassign_count, 1);
    }

    /// 要求テスト項目26: 「前線を設定」ボタンを押すと前線選択モードへ入る。
    /// 再度押すとキャンセル(モード解除)される。
    #[test]
    fn assign_button_toggles_frontline_select_mode() {
        let (mut app, army_id, _fl_id) = build_army_frontline_test_app();
        app.add_systems(Update, handle_army_frontline_command_buttons);

        press_army_frontline_button(&mut app, ArmyFrontlineCommand::Assign);
        app.update();
        assert_eq!(
            app.world().resource::<FrontlineSelectMode>().army_id,
            Some(army_id)
        );

        press_army_frontline_button(&mut app, ArmyFrontlineCommand::Assign);
        app.update();
        assert_eq!(
            app.world().resource::<FrontlineSelectMode>().army_id,
            None,
            "同じボタンを再度押すと選択モードはキャンセルされる"
        );
    }

    /// 要求テスト項目32/33: 「前線を解除」ボタンは選択中Armyだけを現在の前線割当から
    /// 解除し、成功通知を送る。解除してもDivisionの状態は変化しない。
    #[test]
    fn unassign_button_removes_only_selected_army_assignment_and_notifies() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        app.add_systems(Update, handle_army_frontline_command_buttons);

        app.world_mut()
            .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
                world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                    world
                        .resource_mut::<FrontlineRegistry>()
                        .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                        .unwrap();
                });
            });
        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .frontline_for_army(army_id),
            Some(fl_id)
        );

        press_army_frontline_button(&mut app, ArmyFrontlineCommand::Unassign);
        app.update();

        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .frontline_for_army(army_id),
            None
        );

        let division = app
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&DivisionId(1))
            .unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert_eq!(division.destination, None);

        let mut notif_reader = app.world_mut().resource_mut::<Messages<GameNotification>>();
        assert_eq!(
            notif_reader.drain().count(),
            1,
            "解除成功時に通知が1回だけ送られるはず"
        );
    }

    /// 要求テスト項目24: Army未選択時は前線設定/解除ボタンが無効化される
    /// (実行可能色にならない)。
    #[test]
    fn army_frontline_buttons_disabled_without_selection() {
        let (mut app, _army_id, _fl_id) = build_army_frontline_test_app();
        app.world_mut().resource_mut::<SelectedArmy>().0 = None;
        app.add_systems(Update, update_army_frontline_ui);

        app.world_mut().spawn((
            ArmyFrontlineCommandButton(ArmyFrontlineCommand::Assign),
            BackgroundColor(Color::NONE),
        ));
        app.world_mut().spawn((
            ArmyFrontlineCommandButton(ArmyFrontlineCommand::Unassign),
            BackgroundColor(Color::NONE),
        ));
        app.world_mut()
            .spawn((ArmyFrontlineStatusText, Text::new(String::new())));

        app.update();

        let world = app.world_mut();
        let mut query = world.query::<(&ArmyFrontlineCommandButton, &BackgroundColor)>();
        for (btn, bg) in query.iter(world) {
            assert_eq!(
                bg.0, ARMY_GROUP_CMD_DISABLED_COLOR,
                "{:?} must be disabled with no army selected",
                btn.0
            );
        }
    }

    /// 要求テスト項目25: 他国Armyが選択中(通常のUI経路では起こらないが、防御的に検証)の
    /// 場合、前線設定ボタンは実行可能色にならない。
    #[test]
    fn army_frontline_assign_button_disabled_for_foreign_army() {
        let (mut app, _army_id, _fl_id) = build_army_frontline_test_app();
        // 他国(C2)所有のArmyを直接作成し、選択中に差し替える(所有者不問で選択され得る
        // 既存の`SelectedArmy`の性質を模擬)。
        let foreign_army_id = {
            let mut army_registry = app.world_mut().resource_mut::<ArmyRegistry>();
            let mut military_registry = MilitaryRegistry::default();
            let d = Division {
                id: DivisionId(99),
                owner: CountryId(2),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                current_state: StateId(2),
                destination: None,
                current_path: Vec::new(),
                target_state: None,
                manpower: 1000,
                max_manpower: 1000,
                equipment: 10.0,
                max_equipment: 10.0,
                organization: 100.0,
                max_organization: 100.0,
                morale: 1.0,
                max_morale: 1.0,
                experience: 0.0,
                supply_ratio: 1.0,
                movement_progress: 0.0,
                status: DivisionStatus::Idle,
                def_id: DivisionDefinitionId(1),
                attack_power: 10,
                defense_power: 10,
                combat_id: None,
            };
            military_registry.divisions.insert(d.id, d);
            army_registry
                .create_army(CountryId(2), &[DivisionId(99)], &military_registry)
                .unwrap()
        };
        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(foreign_army_id);
        app.add_systems(Update, update_army_frontline_ui);

        app.world_mut().spawn((
            ArmyFrontlineCommandButton(ArmyFrontlineCommand::Assign),
            BackgroundColor(Color::NONE),
        ));
        app.world_mut()
            .spawn((ArmyFrontlineStatusText, Text::new(String::new())));

        app.update();

        let world = app.world_mut();
        let mut query = world.query::<(&ArmyFrontlineCommandButton, &BackgroundColor)>();
        let (_, bg) = query.iter(world).next().unwrap();
        assert_eq!(bg.0, ARMY_GROUP_CMD_DISABLED_COLOR);
    }

    /// 要求テスト項目29: Escapeキーで前線選択モードが解除される(割当は変更しない)。
    #[test]
    fn escape_key_cancels_frontline_select_mode() {
        let (mut app, army_id, _fl_id) = build_army_frontline_test_app();
        app.add_systems(
            Update,
            crate::map::frontline_selection::cancel_frontline_select_mode_on_context_change,
        );
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();

        app.world_mut()
            .resource_mut::<FrontlineSelectMode>()
            .activate(army_id);

        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::Escape);
        app.insert_resource(keys);

        app.update();

        assert_eq!(app.world().resource::<FrontlineSelectMode>().army_id, None);
    }

    /// 要求テスト項目31: 選択中Armyが変わると前線選択モードは自動的にキャンセルされる。
    #[test]
    fn changing_selected_army_cancels_frontline_select_mode() {
        let (mut app, army_id, _fl_id) = build_army_frontline_test_app();
        app.add_systems(
            Update,
            crate::map::frontline_selection::cancel_frontline_select_mode_on_context_change,
        );
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();

        app.world_mut()
            .resource_mut::<FrontlineSelectMode>()
            .activate(army_id);
        app.world_mut().resource_mut::<SelectedArmy>().0 = None;

        app.update();

        assert_eq!(
            app.world().resource::<FrontlineSelectMode>().army_id,
            None,
            "選択中Armyが変わったら前線選択モードは解除されるはず"
        );
    }

    /// 要求テスト項目33/一部45系: `update_army_frontline_ui`が現在の割当を
    /// ステータス行へ反映する(割当済み/未割当/選択中いずれの状態も個別に検証)。
    #[test]
    fn update_army_frontline_ui_reflects_current_assignment_state() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        app.add_systems(Update, update_army_frontline_ui);
        app.world_mut().spawn((
            ArmyFrontlineCommandButton(ArmyFrontlineCommand::Assign),
            BackgroundColor(Color::NONE),
        ));
        app.world_mut().spawn((
            ArmyFrontlineCommandButton(ArmyFrontlineCommand::Unassign),
            BackgroundColor(Color::NONE),
        ));
        app.world_mut()
            .spawn((ArmyFrontlineStatusText, Text::new(String::new())));

        // 未割当の状態
        app.update();
        {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<ArmyFrontlineStatusText>>();
            let text = query.iter(world).next().unwrap();
            assert!(
                text.0.contains("None") || text.0.contains("なし"),
                "未割当時は「なし」系の表示になるはず: {}",
                text.0
            );
        }

        // 割当後
        app.world_mut()
            .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
                world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                    world
                        .resource_mut::<FrontlineRegistry>()
                        .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                        .unwrap();
                });
            });
        app.update();
        {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<ArmyFrontlineStatusText>>();
            let text = query.iter(world).next().unwrap();
            assert!(
                text.0.contains("Dwarf"),
                "割当後は敵国名を含む表示になるはず: {}",
                text.0
            );
        }

        // 選択モード中
        app.world_mut()
            .resource_mut::<FrontlineSelectMode>()
            .activate(army_id);
        app.update();
        {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<ArmyFrontlineStatusText>>();
            let text = query.iter(world).next().unwrap();
            assert!(
                text.0.contains("Selecting") || text.0.contains("選択中"),
                "選択モード中は専用の表示になるはず: {}",
                text.0
            );
        }
    }

    /// 要求テスト項目: `map::frontline_selection::FrontlineAssignmentAttempted`
    /// (割当成功/無効クリック)を購読し、それぞれ異なる通知を送る。
    #[test]
    fn frontline_assignment_attempted_events_produce_distinct_notifications() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        app.add_systems(Update, handle_frontline_assignment_attempted);

        app.world_mut()
            .resource_mut::<Messages<FrontlineAssignmentAttempted>>()
            .write(FrontlineAssignmentAttempted::Assigned {
                army_id,
                frontline_id: fl_id,
            });
        app.update();
        let assigned_message = {
            let mut notif = app.world_mut().resource_mut::<Messages<GameNotification>>();
            notif.drain().next().unwrap().message
        };

        app.world_mut()
            .resource_mut::<Messages<FrontlineAssignmentAttempted>>()
            .write(FrontlineAssignmentAttempted::Invalid);
        app.update();
        let invalid_message = {
            let mut notif = app.world_mut().resource_mut::<Messages<GameNotification>>();
            notif.drain().next().unwrap().message
        };

        assert_ne!(
            assigned_message, invalid_message,
            "割当成功と無効クリックは異なる通知文言であるべき"
        );
    }

    /// 要求テスト項目: Armyパネルを閉じると前線選択モードが解除される。
    #[test]
    fn closing_panel_cancels_frontline_select_mode() {
        let (mut app, army_id, _fl_id) = build_army_frontline_test_app();
        app.add_systems(Update, cancel_frontline_select_mode_on_panel_close);

        app.world_mut()
            .resource_mut::<FrontlineSelectMode>()
            .activate(army_id);
        app.world_mut().resource_mut::<MilitaryPanelState>().open = false;

        app.update();

        assert_eq!(app.world().resource::<FrontlineSelectMode>().army_id, None);
    }

    // ─── P21-007: 攻勢線(計画データのみ)UI ────────────────────────────────────

    fn press_offensive_line_button(app: &mut App, cmd: OffensiveLineCommand) {
        app.world_mut()
            .spawn((OffensiveLineCommandButton(cmd), Interaction::Pressed));
    }

    fn assign_test_army_to_frontline(
        app: &mut App,
        army_id: crate::common::ArmyId,
        fl_id: FrontlineId,
    ) {
        app.world_mut()
            .resource_scope(|world, army_registry: Mut<ArmyRegistry>| {
                world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                    world
                        .resource_mut::<FrontlineRegistry>()
                        .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                        .unwrap();
                });
            });
    }

    /// 要求テスト項目: 攻勢線の設定/解除/確定/キャンセルボタンがUIツリーへspawnされている。
    #[test]
    fn offensive_line_buttons_are_spawned_in_military_panel_ui_tree() {
        let mut app = build_test_app();
        app.add_systems(Startup, setup_military_panel);
        app.update();

        let world = app.world_mut();
        for cmd in [
            OffensiveLineCommand::StartEdit,
            OffensiveLineCommand::Clear,
            OffensiveLineCommand::Confirm,
            OffensiveLineCommand::Cancel,
        ] {
            let count = world
                .query::<&OffensiveLineCommandButton>()
                .iter(world)
                .filter(|b| b.0 == cmd)
                .count();
            assert_eq!(count, 1, "{cmd:?}ボタンはちょうど1つspawnされるはず");
        }
        let status_count = world
            .query::<&OffensiveLineStatusText>()
            .iter(world)
            .count();
        assert_eq!(status_count, 1);
    }

    /// 要求テスト項目: 既存攻勢線の編集開始時は、その内容がdraftとして表示される
    /// (再編集時に現在値が初期draftになる)。
    #[test]
    fn start_edit_populates_draft_with_existing_committed_line() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        app.world_mut()
            .resource_mut::<FrontlineRegistry>()
            .get_plan_mut(fl_id, CountryId(1))
            .unwrap()
            .offensive_line_region_ids = Some(vec![StateId(2)]);
        app.add_systems(Update, handle_offensive_line_command_buttons);

        press_offensive_line_button(&mut app, OffensiveLineCommand::StartEdit);
        app.update();

        let mode = app.world().resource::<OffensiveLineEditMode>();
        assert!(mode.is_active());
        assert_eq!(mode.draft, vec![StateId(2)]);
    }

    /// 要求テスト項目12: 確定前のdraftへの変更は、確定ボタンを押すまで
    /// `FrontlineRegistry`側の攻勢線へ一切反映されない。
    #[test]
    fn draft_changes_do_not_affect_committed_plan_before_confirm() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        app.add_systems(Update, handle_offensive_line_command_buttons);

        press_offensive_line_button(&mut app, OffensiveLineCommand::StartEdit);
        app.update();
        app.world_mut()
            .resource_mut::<OffensiveLineEditMode>()
            .toggle_region(StateId(2));

        // draftを変更しただけでは、まだPlan側は未設定のまま。
        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            None
        );

        press_offensive_line_button(&mut app, OffensiveLineCommand::Confirm);
        app.update();

        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            Some(vec![StateId(2)]),
            "確定ボタンを押した時点で初めてPlanへ反映されるはず"
        );
        assert!(
            !app.world().resource::<OffensiveLineEditMode>().is_active(),
            "確定成功後は編集モードが終了するはず"
        );
    }

    /// 要求テスト項目: 不正なdraft(このFrontlineの敵国支配ではない地域を含む)で
    /// 確定しても、Planは変更されず編集モードは継続する。
    #[test]
    fn confirm_with_invalid_draft_is_noop_and_keeps_editing() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        app.add_systems(Update, handle_offensive_line_command_buttons);

        press_offensive_line_button(&mut app, OffensiveLineCommand::StartEdit);
        app.update();
        // StateId(1)はCountryId(1)自身の領域であり、攻勢線としては無効
        // (クリック経由なら弾かれるが、draftを直接操作して不正状態を模した場合の
        // 確定時再検証を確認する)。
        app.world_mut()
            .resource_mut::<OffensiveLineEditMode>()
            .toggle_region(StateId(1));

        press_offensive_line_button(&mut app, OffensiveLineCommand::Confirm);
        app.update();

        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            None,
            "不正なdraftの確定はPlanを一切変更しないはず"
        );
        assert!(
            app.world().resource::<OffensiveLineEditMode>().is_active(),
            "確定に失敗した場合、編集モードは継続してユーザーが修正できるはず"
        );
    }

    /// 要求テスト項目: 「攻勢線を解除」は確認ダイアログなしで即時解除し、
    /// 編集中であれば編集モードも同時にキャンセルする。
    #[test]
    fn clear_button_clears_committed_value_and_active_edit() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        app.world_mut()
            .resource_mut::<FrontlineRegistry>()
            .get_plan_mut(fl_id, CountryId(1))
            .unwrap()
            .offensive_line_region_ids = Some(vec![StateId(2)]);
        app.add_systems(Update, handle_offensive_line_command_buttons);

        press_offensive_line_button(&mut app, OffensiveLineCommand::StartEdit);
        app.update();
        assert!(app.world().resource::<OffensiveLineEditMode>().is_active());

        press_offensive_line_button(&mut app, OffensiveLineCommand::Clear);
        app.update();

        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            None
        );
        assert!(
            !app.world().resource::<OffensiveLineEditMode>().is_active(),
            "解除と同時に編集モードもキャンセルされるはず"
        );
    }

    /// 要求テスト項目11相当: 「キャンセル」ボタンはdraftを破棄するだけでPlanを変更しない。
    #[test]
    fn cancel_button_discards_draft_without_touching_plan() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        app.add_systems(Update, handle_offensive_line_command_buttons);

        press_offensive_line_button(&mut app, OffensiveLineCommand::StartEdit);
        app.update();
        app.world_mut()
            .resource_mut::<OffensiveLineEditMode>()
            .toggle_region(StateId(2));

        press_offensive_line_button(&mut app, OffensiveLineCommand::Cancel);
        app.update();

        assert!(!app.world().resource::<OffensiveLineEditMode>().is_active());
        assert_eq!(
            app.world()
                .resource::<FrontlineRegistry>()
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            None
        );
    }

    /// 要求テスト項目9のUI側確認: 同じFrontlineを共有する別Armyを選択しても、
    /// 同じ攻勢線状態(州数)が表示される。
    #[test]
    fn status_text_shows_same_line_for_different_army_on_same_frontline() {
        let (mut app, army1_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army1_id, fl_id);
        app.world_mut()
            .resource_mut::<FrontlineRegistry>()
            .get_plan_mut(fl_id, CountryId(1))
            .unwrap()
            .offensive_line_region_ids = Some(vec![StateId(2)]);

        let army2_id = app
            .world_mut()
            .resource_scope(|world, mut mil: Mut<MilitaryRegistry>| {
                let division2 = Division {
                    id: DivisionId(2),
                    owner: CountryId(1),
                    division_type: DivisionType::Infantry,
                    size: DivisionSize::Standard,
                    current_state: StateId(1),
                    destination: None,
                    current_path: Vec::new(),
                    target_state: None,
                    manpower: 1000,
                    max_manpower: 1000,
                    equipment: 10.0,
                    max_equipment: 10.0,
                    organization: 100.0,
                    max_organization: 100.0,
                    morale: 1.0,
                    max_morale: 1.0,
                    experience: 0.0,
                    supply_ratio: 1.0,
                    movement_progress: 0.0,
                    status: DivisionStatus::Idle,
                    def_id: DivisionDefinitionId(1),
                    attack_power: 10,
                    defense_power: 10,
                    combat_id: None,
                };
                mil.divisions.insert(division2.id, division2);
                world
                    .resource_mut::<ArmyRegistry>()
                    .create_army(CountryId(1), &[DivisionId(2)], &mil)
                    .unwrap()
            });
        assign_test_army_to_frontline(&mut app, army2_id, fl_id);

        app.add_systems(Update, update_offensive_line_ui);
        app.world_mut()
            .spawn((OffensiveLineStatusText, Text::new(String::new())));
        for cmd in [
            OffensiveLineCommand::StartEdit,
            OffensiveLineCommand::Clear,
            OffensiveLineCommand::Confirm,
            OffensiveLineCommand::Cancel,
        ] {
            app.world_mut().spawn((
                OffensiveLineCommandButton(cmd),
                BackgroundColor(Color::NONE),
            ));
        }

        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(army1_id);
        app.update();
        let text_army1 = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };

        app.world_mut().resource_mut::<SelectedArmy>().0 = Some(army2_id);
        app.update();
        let text_army2 = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };

        assert_eq!(
            text_army1, text_army2,
            "同じFrontlineを共有する別Armyを選択しても同じ攻勢線状態が表示されるはず"
        );
    }

    /// 要求テスト項目19: 軍事パネルが閉じていても、言語切替時は即座に表示が更新される
    /// (`!state.open && !locale.is_changed()`ガードの後半条件を検証する)。
    #[test]
    fn locale_change_updates_status_text_even_when_panel_closed() {
        let (mut app, _army_id, _fl_id) = build_army_frontline_test_app();
        app.world_mut().resource_mut::<MilitaryPanelState>().open = false;
        app.add_systems(Update, update_offensive_line_ui);
        app.world_mut()
            .spawn((OffensiveLineStatusText, Text::new(String::new())));
        for cmd in [
            OffensiveLineCommand::StartEdit,
            OffensiveLineCommand::Clear,
            OffensiveLineCommand::Confirm,
            OffensiveLineCommand::Cancel,
        ] {
            app.world_mut().spawn((
                OffensiveLineCommandButton(cmd),
                BackgroundColor(Color::NONE),
            ));
        }

        app.update();
        let ja_text = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };

        app.world_mut().resource_mut::<CurrentLocale>().0 = crate::localization::Locale::EnUs;
        app.update();
        let en_text = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };

        assert_ne!(
            ja_text, en_text,
            "パネルが閉じていても言語切替で表示テキストが更新されるはず"
        );
    }

    /// P21-008要求テスト項目23: 攻勢線が実行中(Offensive・未確保Stateへ到達可能)の場合の
    /// 状態表示がJA/ENそれぞれ正しく、切替後も即座に更新される。
    #[test]
    fn offensive_line_in_progress_status_text_updates_on_locale_change() {
        let (mut app, army_id, fl_id) = build_army_frontline_test_app();
        assign_test_army_to_frontline(&mut app, army_id, fl_id);
        // StateId(1)所属のDivisionから見て、StateId(2)(敵国Dwarf所有)は隣接済みで
        // 到達可能。攻勢線として設定しOffensive姿勢にすると「実行中」になるはず。
        app.world_mut()
            .resource_scope(|world, state_registry: Mut<StateRegistry>| {
                world.resource_scope(|world, war_registry: Mut<WarRegistry>| {
                    world
                        .resource_mut::<FrontlineRegistry>()
                        .set_offensive_line(
                            fl_id,
                            CountryId(1),
                            &[StateId(2)],
                            &state_registry,
                            &war_registry,
                        )
                        .unwrap();
                });
            });
        if let Some(plan) = app
            .world_mut()
            .resource_mut::<FrontlineRegistry>()
            .get_plan_mut(fl_id, CountryId(1))
        {
            plan.stance = FrontlineStance::Offensive;
        }

        app.add_systems(Update, update_offensive_line_ui);
        app.world_mut()
            .spawn((OffensiveLineStatusText, Text::new(String::new())));
        for cmd in [
            OffensiveLineCommand::StartEdit,
            OffensiveLineCommand::Clear,
            OffensiveLineCommand::Confirm,
            OffensiveLineCommand::Cancel,
        ] {
            app.world_mut().spawn((
                OffensiveLineCommandButton(cmd),
                BackgroundColor(Color::NONE),
            ));
        }

        app.update();
        let ja_text = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };
        assert!(
            !ja_text.contains(MISSING_KEY_MARKER_PREFIX),
            "欠落キーマーカーを含んではならない: {ja_text}"
        );

        app.world_mut().resource_mut::<CurrentLocale>().0 = crate::localization::Locale::EnUs;
        app.update();
        let en_text = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<&Text, With<OffensiveLineStatusText>>();
            query.iter(world).next().unwrap().0.clone()
        };
        assert!(!en_text.contains(MISSING_KEY_MARKER_PREFIX));

        assert_ne!(
            ja_text, en_text,
            "攻勢線実行中の状態表示もJA/EN切替で即座に更新されるはず"
        );
    }
}
