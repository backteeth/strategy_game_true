use crate::app::game_state::GameState;
use crate::common::{CountryId, DivisionDefinitionId, DivisionId, FrontlineId};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text, t, tf};
use crate::map::division_selection::SelectedDivision;
use crate::military::army::{Army, ArmyRegistry};
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
    FrontlineCommandFeasibility, FrontlineRegistry, FrontlineStance,
    evaluate_frontline_division_command_feasibility,
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

/// P21-004: 編成(Army)ボタンが発行する命令の種類。「対象編成」は常に
/// `ArmyRegistry::target_army_for_selection`が選択中陸軍から動的に決定する
/// (選択中のいずれかの陸軍が既に所属している編成、なければ対象なし)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmyCommand {
    /// 選択中陸軍から新しい編成を作成
    Create,
    /// 選択中陸軍を対象編成へ追加
    AddSelection,
    /// 選択中陸軍を(所属する編成があれば)そこから除外
    RemoveSelection,
    /// 対象編成の全所属師団を選択に反映
    SelectArmy,
    /// 対象編成を解散
    Disband,
}

#[derive(Component)]
pub struct ArmyCommandButton(pub ArmyCommand);

/// P21-004: 編成コマンドボタンに付随する対象編成の表示のTextマーカー。
#[derive(Component)]
pub struct ArmyStatusText;

/// P21-004: 編成一覧の表示のTextマーカー。
#[derive(Component)]
pub struct ArmyListText;

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

            // P21-004: 編成(Army)セクション。「対象編成」は選択中陸軍から動的に
            // 決まる(いずれかが所属する編成)ため、専用のUI要素で編成を選ぶ操作は設けない。
            let (ag_header_text, ag_header_marker) = localized_text(
                &catalog,
                locale.0,
                "military_panel.army_header",
                vec![],
            );
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
                        (
                            ArmyCommand::Create,
                            "military_panel.army_create_button",
                        ),
                        (
                            ArmyCommand::AddSelection,
                            "military_panel.army_add_button",
                        ),
                        (
                            ArmyCommand::RemoveSelection,
                            "military_panel.army_remove_button",
                        ),
                        (
                            ArmyCommand::SelectArmy,
                            "military_panel.army_select_button",
                        ),
                        (
                            ArmyCommand::Disband,
                            "military_panel.army_disband_button",
                        ),
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

            parent.spawn((
                ArmyListText,
                Text::new(t(
                    &catalog,
                    locale.0,
                    "military_panel.army_list_empty",
                )),
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
            ));

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
        let assigned_ids = plan.map(|p| p.assigned_division_ids.as_slice()).unwrap_or(&[]);

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
                    format!("{:.0}", division.organization / division.max_organization * 100.0),
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
        let fl_tag = if frontline_registry.division_frontline_map.contains_key(&division.id) {
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

/// P21-004: 選択中陸軍から新しい編成を作成する。所有者不一致・撃破済み陸軍は
/// `ArmyRegistry::create_army`が黙って除外する。
fn execute_army_create(
    army_registry: &mut ArmyRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    army_registry.create_army(player_cid, selected_division_ids, military_registry);
}

/// P21-004: 選択中陸軍を対象編成(選択中のいずれかが既に所属する編成)へ追加する。
fn execute_army_add_selection(
    army_registry: &mut ArmyRegistry,
    military_registry: &MilitaryRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    let Some(target) = army_registry.target_army_for_selection(selected_division_ids) else {
        return;
    };
    for &division_id in selected_division_ids {
        let _ = army_registry.add_division(target, division_id, player_cid, military_registry);
    }
}

/// P21-004: 選択中陸軍を、それぞれの所属編成(あれば)から除外する(未所属へ戻す)。
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

/// P21-004: 対象編成の全所属師団を選択(`SelectedDivision`)に反映する(「軍を選択」ボタン)。
fn execute_army_select(
    army_registry: &ArmyRegistry,
    selected_division: &mut SelectedDivision,
    selected_division_ids: &[DivisionId],
) {
    let Some(target) = army_registry.target_army_for_selection(selected_division_ids) else {
        return;
    };
    if let Some(group) = army_registry.armies.get(&target) {
        for &division_id in &group.member_division_ids {
            selected_division.division_ids.insert(division_id);
        }
    }
}

/// P21-004: 対象編成を解散する。所属していた師団は全員未所属へ戻る。
fn execute_army_disband(
    army_registry: &mut ArmyRegistry,
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
) {
    let Some(target) = army_registry.target_army_for_selection(selected_division_ids) else {
        return;
    };
    let _ = army_registry.disband(target, player_cid);
}

/// P21-004: 編成コマンドボタンの背景色・対象編成表示・編成一覧を更新する。
/// `update_military_panel_ui`はSystemParamタプル引数数上限に近いため
/// (`update_recruit_button_ui`/`update_frontline_command_buttons_ui`と同じ理由)、
/// 独立したSystemとして切り出す。
#[allow(clippy::too_many_arguments)]
fn update_army_ui(
    state: Res<MilitaryPanelState>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    army_registry: Res<ArmyRegistry>,
    selected_division: Res<SelectedDivision>,
    loc: crate::localization::Loc,
    mut btn_q: Query<(&ArmyCommandButton, &mut BackgroundColor)>,
    mut status_text_q: Query<&mut Text, (With<ArmyStatusText>, Without<ArmyListText>)>,
    mut list_text_q: Query<&mut Text, (With<ArmyListText>, Without<ArmyStatusText>)>,
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

    let selected_ids = selected_division.sorted_ids();
    let is_own = |id: &DivisionId| {
        military_registry
            .divisions
            .get(id)
            .map(|a| a.owner == player_cid)
            .unwrap_or(false)
    };
    let has_own_selection = selected_ids.iter().any(is_own);
    let target_group = army_registry.target_army_for_selection(&selected_ids);
    let has_selection_outside_target = selected_ids
        .iter()
        .any(|id| is_own(id) && army_registry.army_for_division(*id) != target_group);
    let has_grouped_selection = selected_ids
        .iter()
        .any(|id| army_registry.army_for_division(*id).is_some());

    let can_create = has_own_selection;
    let can_add = target_group.is_some() && has_selection_outside_target;
    let can_remove = has_grouped_selection;
    let can_select = target_group.is_some();
    let can_disband = target_group.is_some();

    for (btn, mut bg) in btn_q.iter_mut() {
        let ready = match btn.0 {
            ArmyCommand::Create => can_create,
            ArmyCommand::AddSelection => can_add,
            ArmyCommand::RemoveSelection => can_remove,
            ArmyCommand::SelectArmy => can_select,
            ArmyCommand::Disband => can_disband,
        };
        *bg = BackgroundColor(if ready {
            ARMY_GROUP_CMD_READY_COLOR
        } else {
            ARMY_GROUP_CMD_DISABLED_COLOR
        });
    }

    let status_line = target_group
        .and_then(|group_id| army_registry.armies.get(&group_id))
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

    let mut groups: Vec<&Army> = army_registry
        .armies
        .values()
        .filter(|g| g.owner == player_cid)
        .collect();
    groups.sort_by_key(|g| g.id.0);

    let list_text = if groups.is_empty() {
        tr("military_panel.army_list_empty")
    } else {
        let mut lines = vec![trf(
            "military_panel.army_list_header",
            vec![("count", groups.len().to_string())],
        )];
        for group in groups {
            let ids = group
                .member_division_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(trf(
                "military_panel.army_list_line",
                vec![
                    ("name", group.name.clone()),
                    ("count", group.member_division_ids.len().to_string()),
                    ("ids", ids),
                ],
            ));
        }
        lines.join("\n")
    };
    if let Ok(mut text) = list_text_q.single_mut()
        && text.0 != list_text
    {
        text.0 = list_text;
    }
}

/// P21-004: 編成コマンドボタン(作成/追加/除外/軍を選択/解散)のクリックを処理する。
/// `Changed<Interaction>`+`Pressed`によりクリック1回につき1回だけ命令を発行する
/// (前線命令ボタンと同型のパターン)。
fn handle_army_command_buttons(
    btn_q: Query<(&Interaction, &ArmyCommandButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    military_registry: Res<MilitaryRegistry>,
    mut selected_division: ResMut<SelectedDivision>,
    mut army_registry: ResMut<ArmyRegistry>,
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
            ),
            ArmyCommand::AddSelection => execute_army_add_selection(
                &mut army_registry,
                &military_registry,
                &selected_ids,
                player_cid,
            ),
            ArmyCommand::RemoveSelection => execute_army_remove_selection(
                &mut army_registry,
                &military_registry,
                &selected_ids,
                player_cid,
            ),
            ArmyCommand::SelectArmy => {
                execute_army_select(&army_registry, &mut selected_division, &selected_ids)
            }
            ArmyCommand::Disband => {
                execute_army_disband(&mut army_registry, &selected_ids, player_cid)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::StateId;
    use crate::country::CountryData;
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
        let (mut app, own_division_id, _enemy_division_id, fl_id) = build_frontline_command_test_app();
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Assign);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert_eq!(
            frontline_registry.division_frontline_map.get(&own_division_id),
            Some(&fl_id)
        );
    }

    /// P21-002最重要回帰テスト: 選択中陸軍が敵国のものであっても
    /// (`SelectedDivision`は所有者を問わず選択され得るため)、解除ボタンを押しても
    /// 敵国の前線割当は一切変化しない(6-1のバグ修正確認)。
    #[test]
    fn handle_frontline_command_buttons_unassign_rejects_foreign_division() {
        let (mut app, _own_division_id, enemy_division_id, fl_id) = build_frontline_command_test_app();
        // 選択中陸軍を敵国のものに差し替える(左クリックで選択され得る状態を再現)
        app.world_mut()
            .resource_mut::<SelectedDivision>()
            .select_only(enemy_division_id);
        app.add_systems(Update, handle_frontline_command_buttons);
        press_frontline_command_button(&mut app, FrontlineCommand::Unassign);

        app.update();

        let frontline_registry = app.world().resource::<FrontlineRegistry>();
        assert_eq!(
            frontline_registry.division_frontline_map.get(&enemy_division_id),
            Some(&fl_id),
            "unassign button must not remove another country's frontline assignment"
        );
        let enemy_plan = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();
        assert!(enemy_plan.assigned_division_ids.contains(&enemy_division_id));
    }

    #[test]
    fn handle_frontline_command_buttons_unassign_all_only_affects_own_plan() {
        let (mut app, own_division_id, enemy_division_id, fl_id) = build_frontline_command_test_app();
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
            frontline_registry.division_frontline_map.get(&enemy_division_id),
            Some(&fl_id),
            "enemy country's plan must not be affected by the player's unassign-all"
        );
    }

    #[test]
    fn handle_frontline_command_buttons_set_stance() {
        let (mut app, _own_division_id, _enemy_division_id, fl_id) = build_frontline_command_test_app();
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
        let (mut app, own_division_id, _enemy_division_id, _fl_id) = build_frontline_command_test_app();
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
        let (mut app, own_division_id, _enemy_division_id, fl_id) = build_frontline_command_test_app();
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
        app.insert_resource(PlayerCountry(Some(player_cid)));
        app.insert_resource(SelectedDivision {
            division_ids: [own_division_1].into_iter().collect(),
        });

        (app, own_division_1, own_division_2, enemy_division)
    }

    fn press_army_command_button(app: &mut App, cmd: ArmyCommand) {
        app.world_mut()
            .spawn((ArmyCommandButton(cmd), Interaction::Pressed));
    }

    /// UI接続確認: 編成コマンドボタン5個(作成/追加/除外/軍を選択/解散)と、
    /// 対象編成表示・編成一覧のテキストが軍事パネルUIツリーへspawnされていること。
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
            count, 5,
            "all 5 division group command buttons must be present in the spawned military panel UI tree"
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
                .query::<&ArmyListText>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[test]
    fn handle_army_command_buttons_create_success() {
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1, own_division_2].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Create);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(registry.armies.len(), 1);
        let group = registry.armies.values().next().unwrap();
        assert_eq!(group.member_division_ids, vec![own_division_1, own_division_2]);
        assert_eq!(group.owner, CountryId(1));
    }

    /// P21-003監査で発見した「選択が所有者を問わない」不具合が編成側に波及しないことの
    /// 回帰テスト: 敵国陸軍が選択に混ざっていても、編成には自国陸軍だけが入る。
    #[test]
    fn handle_army_command_buttons_create_ignores_foreign_division() {
        let (mut app, own_division_1, _own_division_2, enemy_division) = build_army_command_test_app();
        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1, enemy_division].into_iter().collect();
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
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1]);

        // 既存グループ所属の師団1 + 未所属の師団2 を選択してAdd
        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1, own_division_2].into_iter().collect();
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
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::RemoveSelection);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert_eq!(registry.army_for_division(own_division_1), None);
        assert_eq!(registry.armies[&group_id].member_division_ids, vec![own_division_2]);
    }

    #[test]
    fn handle_army_command_buttons_select_group_expands_selection() {
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        // 師団1だけを選択した状態から「軍を選択」を押す
        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::SelectArmy);

        app.update();

        let selected = app.world().resource::<SelectedDivision>();
        assert_eq!(selected.len(), 2);
        assert!(selected.is_selected(own_division_1));
        assert!(selected.is_selected(own_division_2));
    }

    #[test]
    fn handle_army_command_buttons_disband_returns_members_to_unassigned() {
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        let group_id = create_test_group(&mut app, CountryId(1), &[own_division_1, own_division_2]);

        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1].into_iter().collect();
        app.add_systems(Update, handle_army_command_buttons);
        press_army_command_button(&mut app, ArmyCommand::Disband);

        app.update();

        let registry = app.world().resource::<ArmyRegistry>();
        assert!(!registry.armies.contains_key(&group_id));
        assert_eq!(registry.army_for_division(own_division_1), None);
        assert_eq!(registry.army_for_division(own_division_2), None);
    }

    #[test]
    fn handle_army_command_buttons_fires_once_per_press() {
        let (mut app, own_division_1, own_division_2, _enemy_division) = build_army_command_test_app();
        app.world_mut().resource_mut::<SelectedDivision>().division_ids =
            [own_division_1, own_division_2].into_iter().collect();
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
}
