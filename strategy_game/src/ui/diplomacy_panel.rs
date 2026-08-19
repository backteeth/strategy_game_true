use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId};
use crate::country::power::CountryPowerRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::{ClaimRegistry, ClaimSource};
use crate::diplomacy::crisis::{CrisisPhase, CrisisRegistry, ThirdCountryReaction};
use crate::diplomacy::crisis_response;
use crate::diplomacy::data::{
    ActiveDiplomaticActivity, ActiveTreaty, DiplomacyRegistry, DiplomaticActivityType, TreatyType,
};
use crate::diplomacy::proposal::calculate_proposal_score;
use crate::diplomacy::update::ACTIVITY_DURATION_DAYS;
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::ui::tab_bar::{TabBarRoot, ctrl_held};
use bevy::prelude::*;

use crate::war::data::WarRegistry;
use crate::war::justification::WarJustificationRegistry;

#[derive(Component)]
pub struct DiplomacyPanelRoot;

#[derive(Resource, Default)]
pub struct DiplomacyPanelState {
    pub open: bool,
    pub target_country: Option<CountryId>,
    /// P21-010: 請求対象として選択中の州(対象国が所有する陸上州のみ有効)。
    /// 対象国変更・パネルを閉じる・ロード・`GameState::Playing`離脱で破棄する。
    pub claim_target_state: Option<StateId>,
    /// P21-010: 危機開始の確認待ちClaim(インライン2段階確認の1段階目)。
    /// `claim_target_state`と同じタイミングで破棄する。
    pub pending_crisis_claim: Option<ClaimId>,
    /// P21-011: 最後通牒受諾の確認待ちCrisis(インライン2段階確認の1段階目)。
    pub pending_crisis_accept: Option<DiplomaticCrisisId>,
    /// P21-011: 最後通牒拒否の確認待ちCrisis(インライン2段階確認の1段階目)。
    pub pending_crisis_reject: Option<DiplomaticCrisisId>,
    /// P21-013: 第三国支持表明の確認待ちCrisis・陣営(インライン2段階確認の1段階目)。
    pub pending_support_pledge: Option<(DiplomaticCrisisId, crisis_response::CrisisSupportSide)>,
}

impl DiplomacyPanelState {
    /// P21-010/011/013: 一時的な請求州選択・危機開始/受諾/拒否/支持確認状態だけを破棄する
    /// (`open`/`target_country`は対象外)。
    fn clear_transient_selection(&mut self) {
        self.claim_target_state = None;
        self.pending_crisis_claim = None;
        self.pending_crisis_accept = None;
        self.pending_crisis_reject = None;
        self.pending_support_pledge = None;
    }
}

#[derive(Component)]
pub struct ToggleDiplomacyPanelButton;

#[derive(Component)]
pub struct ImproveRelationsButton(pub CountryId);

#[derive(Component)]
pub struct HarmRelationsButton(pub CountryId);

#[derive(Component)]
pub struct ProposeTreatyButton(pub CountryId, pub TreatyType);

#[derive(Component)]
pub struct BreakTreatyButton(pub CountryId, pub TreatyType);

#[derive(Component)]
pub struct JustifyWarButton(pub CountryId, pub crate::common::StateId);

#[derive(Component)]
pub struct DeclareWarButton(pub CountryId, pub crate::common::StateId);

/// P21-010: Claimパネル操作が発行する命令の種類。`Create`は選択中の
/// `target_country`/`claim_target_state`を実行時に読み直すだけで(payloadを持たない)、
/// 同一フレーム内で対象国が切り替わった場合でも古い対象へ作成しない
/// (`can_create_claim`が実行時点の対象国と州所有者を必ず再検証する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimCommand {
    SelectState(StateId),
    Create,
}

#[derive(Component)]
pub struct ClaimCommandButton(pub ClaimCommand);

/// P21-010: Crisisパネル操作が発行する命令の種類。`Confirm`/`Cancel`はpayloadを持たず、
/// 実行時点の`pending_crisis_claim`を読み直す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrisisCommand {
    RequestStart(ClaimId),
    Confirm,
    Cancel,
    /// P21-011: 対象国による最後通牒受諾の要求(1段階目)。
    RequestAccept(DiplomaticCrisisId),
    ConfirmAccept,
    CancelAccept,
    /// P21-011: 対象国による最後通牒拒否の要求(1段階目)。
    RequestReject(DiplomaticCrisisId),
    ConfirmReject,
    CancelReject,
    /// P21-011: initiatorによる撤回。確認なしの単発操作
    /// (受諾/拒否のような不可逆な領土・正当化の変更を伴わないため)。
    Withdraw(DiplomaticCrisisId),
    /// P21-013: 第三国による支持表明の要求(1段階目)。要求国側/対象国側。
    RequestSupportInitiator(DiplomaticCrisisId),
    RequestSupportTarget(DiplomaticCrisisId),
    ConfirmSupport,
    CancelSupport,
    /// P21-013: 第三国による支持撤回。確認なしの単発操作(`Withdraw`と同じ理由:
    /// 領土・正当化のような不可逆な変更を伴わない)。
    WithdrawSupport(DiplomaticCrisisId),
}

#[derive(Component)]
pub struct CrisisCommandButton(pub CrisisCommand);

#[derive(Component)]
pub struct DiplomacyHeaderText;

/// P21-014: 国家総合力・国家ランク表示専用のコンテナ。`update_diplomacy_panel_ui`が
/// 毎回全破棄・再構築する`DiplomacyContentContainer`とは独立に、専用の
/// `update_country_power_info_ui`だけが管理する(互いの子を despawn し合わない)。
#[derive(Component)]
pub struct CountryPowerInfoContainer;

#[derive(Component)]
pub struct DiplomacyContentContainer;

pub struct DiplomacyPluginUI;

impl Plugin for DiplomacyPluginUI {
    fn build(&self, app: &mut App) {
        app.insert_resource(DiplomacyPanelState::default())
            .add_systems(
                OnEnter(GameState::Playing),
                setup_diplomacy_panel.after(crate::ui::tab_bar::spawn_tab_bar),
            )
            .add_systems(
                Update,
                (
                    toggle_diplomacy_panel_key,
                    sync_target_country_from_selected_state,
                    handle_diplomacy_action_buttons,
                    handle_claim_command_buttons,
                    handle_crisis_command_buttons,
                    // P21-013: 同一フレームでの日付変更(AI日次応答)との競合裁定のため、
                    // `handle_daily_diplomacy`より後に明示的に配置する(関数doc参照)。
                    handle_crisis_support_command_buttons
                        .after(crate::diplomacy::update::handle_daily_diplomacy),
                    update_diplomacy_panel_ui,
                    update_country_power_info_ui,
                )
                    .chain()
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                OnExit(GameState::Playing),
                reset_diplomacy_panel_transient_state,
            );
    }
}

/// P21-010: `GameState::Playing`離脱時に一時的な請求州選択・危機開始確認状態を破棄する。
fn reset_diplomacy_panel_transient_state(mut state: ResMut<DiplomacyPanelState>) {
    state.clear_transient_selection();
}

fn setup_diplomacy_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    tab_bar_q: Query<Entity, With<TabBarRoot>>,
) {
    // タブバー共通コンテナの子としてトグルボタンを配置(`ui::tab_bar`参照)
    if let Ok(tab_bar) = tab_bar_q.single() {
        commands.entity(tab_bar).with_children(|parent| {
            parent
                .spawn((
                    ToggleDiplomacyPanelButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.2, 0.4, 0.3, 0.9)),
                ))
                .with_children(|btn| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "diplomacy_panel.toggle_button", vec![]);
                    btn.spawn((
                        text,
                        marker,
                        TextColor(Color::WHITE),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                });
        });
    }

    // タイトルはスクロール対象の外(常に表示)、それ以外はスクロール可能な本体
    // (`ui::scroll::spawn_scrollable_body`)へ入れる(詳細は`ui::scroll`のドキュメント参照)。
    let diplomacy_panel_entity = commands
        .spawn((
            DiplomacyPanelRoot,
            // P21-013: 背景自体もButton化し、子Button以外の余白をクリック/ホバーしても
            // `Interaction`を確実に発行させる(`ui::load_confirm`の既存パターンを踏襲)。
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(75.0),
                width: Val::Px(600.0),
                height: Val::Px(600.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.1, 0.08, 0.95)),
        ))
        .id();
    commands
        .entity(diplomacy_panel_entity)
        .with_children(|parent| {
            let (text, marker) =
                localized_text(&catalog, locale.0, "diplomacy_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.6, 0.9, 0.7)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            crate::ui::scroll::spawn_scrollable_body(parent, Val::Px(8.0), |parent| {
                parent.spawn((
                    DiplomacyHeaderText,
                    Text::new(""),
                    LocalizedText::default(),
                    TextColor(Color::srgb(0.85, 0.85, 0.9)),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                ));

                // P21-014: 国家総合力/国家ランク表示。P21-013の支持UI(クライシス一覧)より
                // 上、国家基本情報(見出し)の直後に配置する。
                parent.spawn((
                    CountryPowerInfoContainer,
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(2.0),
                        ..default()
                    },
                ));

                parent.spawn((
                    DiplomacyContentContainer,
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(6.0),
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
            });
        });
}

fn toggle_diplomacy_panel_key(
    mut state: ResMut<DiplomacyPanelState>,
    mut active_panel: ResMut<crate::ui::ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<ToggleDiplomacyPanelButton>, Changed<Interaction>)>,
    mut panel_q: Query<&mut Node, With<DiplomacyPanelRoot>>,
) {
    let mut toggle = false;
    // Ctrl+3: タブ切替は全パネル共通でCtrl+数字に統一
    // (軍事パネル内の素のDigit1/2/3/7/8/9フロントライン操作キーとの衝突を避けるため)。
    if keys.just_pressed(KeyCode::Digit3) && ctrl_held(&keys) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        // 他パネル(研究/内政/軍事)と同じくActivePanelを介して排他制御する。
        // 従来はこのパネル単独のopenフラグでNode.displayを直接書き換えていたため、
        // 他パネルを開閉した際にsync_panels_to_active(ui/mod.rs)がActivePanelの
        // 変化を検知してこのパネルのdisplayをNoneへ強制上書きし、openフラグとの
        // 食い違いが発生していた。
        active_panel.toggle(crate::ui::PanelKind::Diplomacy);
        state.open = active_panel.current == crate::ui::PanelKind::Diplomacy;
        if !state.open {
            // P21-010: パネルを閉じたら一時的な請求州選択・危機開始確認状態を破棄する。
            state.clear_transient_selection();
        }
        if let Ok(mut node) = panel_q.single_mut() {
            node.display = if state.open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

fn sync_target_country_from_selected_state(
    selected_state: Res<SelectedState>,
    state_registry: Res<StateRegistry>,
    mut diplo_state: ResMut<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
) {
    if !selected_state.is_changed() {
        return;
    }
    if let Some(state) = selected_state.0.and_then(|sid| state_registry.get(sid))
        && player_country.0 != Some(state.owner_country_id)
        && diplo_state.target_country != Some(state.owner_country_id)
    {
        diplo_state.target_country = Some(state.owner_country_id);
        // P21-010: 対象国が変わったら一時的な請求州選択・危機開始確認状態を破棄する
        // (古い対象国向けの選択を新しい対象国へ持ち越さない)。
        diplo_state.clear_transient_selection();
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_diplomacy_action_buttons(
    imp_q: Query<(&Interaction, &ImproveRelationsButton), Changed<Interaction>>,
    harm_q: Query<(&Interaction, &HarmRelationsButton), Changed<Interaction>>,
    prop_q: Query<(&Interaction, &ProposeTreatyButton), Changed<Interaction>>,
    break_q: Query<(&Interaction, &BreakTreatyButton), Changed<Interaction>>,
    just_q: Query<(&Interaction, &JustifyWarButton), Changed<Interaction>>,
    dec_q: Query<(&Interaction, &DeclareWarButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut diplo_registry: ResMut<DiplomacyRegistry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
    date: Res<GameDate>,
    mut war_registry: ResMut<WarRegistry>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    // P21-011: crisis_registry追加でSystemParamの16個上限に達したため、
    // locale/catalogを`Loc`(既存の共通ヘルパー、`localization::Loc`のdoc参照)へ統合。
    loc: crate::localization::Loc,
) {
    let Some(p_cid) = player_country.0 else {
        return;
    };
    let Some(proposer) = country_registry.get(p_cid) else {
        return;
    };

    for (interaction, btn) in imp_q.iter() {
        if *interaction == Interaction::Pressed {
            let target_cid = btn.0;
            if let Some(rel) = diplo_registry.get_or_create_mut(p_cid, target_cid) {
                rel.active_activity = Some(ActiveDiplomaticActivity {
                    activity_type: DiplomaticActivityType::ImproveRelations,
                    initiator: p_cid,
                    target: target_cid,
                    days_remaining: ACTIVITY_DURATION_DAYS,
                    daily_opinion_change: 1.0,
                });
                notif_writer.write(GameNotification {
                    message: tf(
                        &loc.catalog,
                        loc.locale.0,
                        "diplomacy_panel.notif_diplomacy_started_improve",
                        vec![("id", target_cid.0.to_string())],
                    ),
                });
            }
        }
    }

    for (interaction, btn) in harm_q.iter() {
        if *interaction == Interaction::Pressed {
            let target_cid = btn.0;
            if let Some(rel) = diplo_registry.get_or_create_mut(p_cid, target_cid) {
                rel.active_activity = Some(ActiveDiplomaticActivity {
                    activity_type: DiplomaticActivityType::HarmRelations,
                    initiator: p_cid,
                    target: target_cid,
                    days_remaining: ACTIVITY_DURATION_DAYS,
                    daily_opinion_change: -1.0,
                });
                notif_writer.write(GameNotification {
                    message: tf(
                        &loc.catalog,
                        loc.locale.0,
                        "diplomacy_panel.notif_diplomacy_started_harm",
                        vec![("id", target_cid.0.to_string())],
                    ),
                });
            }
        }
    }

    for (interaction, btn) in prop_q.iter() {
        if *interaction == Interaction::Pressed {
            let target_cid = btn.0;
            let treaty_type = btn.1;

            if let Some(target) = country_registry.get(target_cid) {
                let rel = diplo_registry.get_or_default(p_cid, target_cid);
                let breakdown =
                    calculate_proposal_score(treaty_type, proposer, target, &rel, &state_registry);

                if breakdown.accepted {
                    let rel_mut = diplo_registry.get_or_create_mut(p_cid, target_cid).unwrap();
                    rel_mut.treaties.push(ActiveTreaty {
                        treaty_type,
                        countries: (p_cid, target_cid),
                        signed_date: date.display(),
                        is_active: true,
                    });
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_proposal_accepted",
                            vec![
                                (
                                    "treaty",
                                    t(&loc.catalog, loc.locale.0, treaty_type.display_name()),
                                ),
                                ("country", target.name.clone()),
                            ],
                        ),
                    });
                } else {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_proposal_rejected",
                            vec![
                                (
                                    "treaty",
                                    t(&loc.catalog, loc.locale.0, treaty_type.display_name()),
                                ),
                                ("country", target.name.clone()),
                            ],
                        ),
                    });
                }
            }
        }
    }

    for (interaction, btn) in break_q.iter() {
        if *interaction == Interaction::Pressed {
            let target_cid = btn.0;
            let treaty_type = btn.1;
            if let Some(rel) = diplo_registry.get_mut(p_cid, target_cid)
                && rel.remove_treaty(treaty_type)
            {
                rel.opinion = (rel.opinion - 25.0).clamp(-100.0, 100.0);
                notif_writer.write(GameNotification {
                    message: tf(
                        &loc.catalog,
                        loc.locale.0,
                        "diplomacy_panel.notif_treaty_broken",
                        vec![
                            (
                                "treaty",
                                t(&loc.catalog, loc.locale.0, treaty_type.display_name()),
                            ),
                            ("id", target_cid.0.to_string()),
                        ],
                    ),
                });
            }
        }
    }

    for (interaction, btn) in just_q.iter() {
        if *interaction == Interaction::Pressed {
            match justification_registry.start_justification(
                p_cid,
                btn.0,
                btn.1,
                date.display(),
                &country_registry,
                &state_registry,
                &diplo_registry,
            ) {
                Ok(_) => {
                    let st_name = state_registry
                        .get(btn.1)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| t(&loc.catalog, loc.locale.0, "common.unknown"));
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_justification_started",
                            vec![("state", st_name)],
                        ),
                    });
                }
                Err(err) => {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_justify_failed",
                            vec![("reason", t(&loc.catalog, loc.locale.0, err))],
                        ),
                    });
                }
            }
        }
    }

    for (interaction, btn) in dec_q.iter() {
        if *interaction == Interaction::Pressed {
            // P21-011: declare_warが正当化を消費(削除)する前に、現在準備完了している
            // justificationのidを捕捉しておく。Crisis連携(sync_crisis_on_war_declared)は
            // このidを使ってEscalating中のCrisisを照合するため、消費後では手遅れになる。
            let ready_justification_id = justification_registry
                .get_ready_justification(p_cid, btn.0, btn.1)
                .map(|j| j.id);

            match war_registry.declare_war(
                p_cid,
                btn.0,
                btn.1,
                date.display(),
                &country_registry,
                &state_registry,
                &mut diplo_registry,
                &mut justification_registry,
            ) {
                Ok(war_id) => {
                    if let Some(j_id) = ready_justification_id {
                        crisis_response::sync_crisis_on_war_declared(
                            &mut crisis_registry,
                            p_cid,
                            btn.0,
                            j_id,
                            war_id,
                        );
                    }
                    let target_name = country_registry
                        .get(btn.0)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| t(&loc.catalog, loc.locale.0, "common.unknown"));
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_war_declared",
                            vec![
                                ("country", target_name),
                                ("war_id", format!("{:?}", war_id)),
                            ],
                        ),
                    });
                }
                Err(err) => {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &loc.catalog,
                            loc.locale.0,
                            "diplomacy_panel.notif_declare_failed",
                            vec![("reason", t(&loc.catalog, loc.locale.0, err))],
                        ),
                    });
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// P21-010: 領土請求(Claim)コマンド処理
// ─────────────────────────────────────────────────────────────────────────

fn execute_claim_select_state(diplo_state: &mut DiplomacyPanelState, state_id: StateId) {
    diplo_state.claim_target_state = Some(state_id);
    // 州選択をやり直したら、古い選択に紐づく危機開始確認は破棄する。
    diplo_state.pending_crisis_claim = None;
}

#[allow(clippy::too_many_arguments)]
fn execute_claim_create(
    diplo_state: &mut DiplomacyPanelState,
    claim_registry: &mut ClaimRegistry,
    player: CountryId,
    target: CountryId,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    let Some(target_state) = diplo_state.claim_target_state else {
        return;
    };

    match claim_registry.create_claim(
        player,
        target,
        target_state,
        date.display(),
        ClaimSource::Strategic,
        country_registry,
        state_registry,
    ) {
        Ok(_) => {
            let st_name = state_registry
                .get(target_state)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| t(catalog, locale.0, "common.unknown"));
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_claim_created",
                    vec![("state", st_name)],
                ),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_claim_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_claim_command_buttons(
    btn_q: Query<(&Interaction, &ClaimCommandButton), Changed<Interaction>>,
    mut diplo_state: ResMut<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
    mut claim_registry: ResMut<ClaimRegistry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    date: Res<GameDate>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player) = player_country.0 else {
        return;
    };
    let Some(target) = diplo_state.target_country else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            ClaimCommand::SelectState(state_id) => {
                execute_claim_select_state(&mut diplo_state, state_id)
            }
            ClaimCommand::Create => execute_claim_create(
                &mut diplo_state,
                &mut claim_registry,
                player,
                target,
                &country_registry,
                &state_registry,
                &date,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// P21-010: 外交危機(Crisis)コマンド処理
// ─────────────────────────────────────────────────────────────────────────

fn execute_crisis_request_start(diplo_state: &mut DiplomacyPanelState, claim_id: ClaimId) {
    diplo_state.pending_crisis_claim = Some(claim_id);
}

fn execute_crisis_cancel(diplo_state: &mut DiplomacyPanelState) {
    diplo_state.pending_crisis_claim = None;
}

/// P21-011: 受諾/拒否の要求(1段階目)。もう一方の確認待ち状態は同時に破棄する
/// (同時に2つの確認プロンプトを表示させないため)。
fn execute_crisis_request_accept(
    diplo_state: &mut DiplomacyPanelState,
    crisis_id: DiplomaticCrisisId,
) {
    diplo_state.pending_crisis_accept = Some(crisis_id);
    diplo_state.pending_crisis_reject = None;
}

fn execute_crisis_cancel_accept(diplo_state: &mut DiplomacyPanelState) {
    diplo_state.pending_crisis_accept = None;
}

fn execute_crisis_request_reject(
    diplo_state: &mut DiplomacyPanelState,
    crisis_id: DiplomaticCrisisId,
) {
    diplo_state.pending_crisis_reject = Some(crisis_id);
    diplo_state.pending_crisis_accept = None;
}

fn execute_crisis_cancel_reject(diplo_state: &mut DiplomacyPanelState) {
    diplo_state.pending_crisis_reject = None;
}

/// P21-011: 検証込みで最後通牒を受諾する(確認の2段階目)。
#[allow(clippy::too_many_arguments)]
fn execute_crisis_confirm_accept(
    diplo_state: &mut DiplomacyPanelState,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    state_registry: &mut StateRegistry,
    responder: CountryId,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    let Some(crisis_id) = diplo_state.pending_crisis_accept else {
        return;
    };
    // 確認は必ずここで閉じる(成功/失敗いずれの場合も、確認待ち状態を持ち越さない)。
    diplo_state.pending_crisis_accept = None;

    match crisis_response::accept_demand(
        crisis_registry,
        claim_registry,
        state_registry,
        crisis_id,
        responder,
    ) {
        Ok(()) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_crisis_accepted"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_crisis_accept_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

/// P21-011: 検証込みで最後通牒を拒否する(確認の2段階目)。
#[allow(clippy::too_many_arguments)]
fn execute_crisis_confirm_reject(
    diplo_state: &mut DiplomacyPanelState,
    crisis_registry: &mut CrisisRegistry,
    justification_registry: &mut WarJustificationRegistry,
    responder: CountryId,
    date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    let Some(crisis_id) = diplo_state.pending_crisis_reject else {
        return;
    };
    diplo_state.pending_crisis_reject = None;

    match crisis_response::reject_demand(
        crisis_registry,
        justification_registry,
        crisis_id,
        responder,
        date.display(),
    ) {
        Ok(()) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_crisis_rejected"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_crisis_reject_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

/// P21-011: initiatorによるCrisis撤回(確認なしの単発操作)。
fn execute_crisis_withdraw(
    crisis_registry: &mut CrisisRegistry,
    justification_registry: &mut WarJustificationRegistry,
    crisis_id: DiplomaticCrisisId,
    initiator: CountryId,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    match crisis_response::withdraw_crisis(
        crisis_registry,
        justification_registry,
        crisis_id,
        initiator,
    ) {
        Ok(()) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_crisis_withdrawn"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_crisis_withdraw_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// P21-013: 第三国Crisis支持コマンド処理
// ─────────────────────────────────────────────────────────────────────────

/// P21-013: 支持表明の要求(1段階目)。他の確認待ち状態は同時に破棄する
/// (同時に2つの確認プロンプトを表示させないため、P21-011の受諾/拒否要求と同じ規約)。
fn execute_support_request(
    diplo_state: &mut DiplomacyPanelState,
    crisis_id: DiplomaticCrisisId,
    side: crisis_response::CrisisSupportSide,
) {
    diplo_state.pending_support_pledge = Some((crisis_id, side));
}

fn execute_support_cancel(diplo_state: &mut DiplomacyPanelState) {
    diplo_state.pending_support_pledge = None;
}

/// P21-013: 検証込みで支持を確定する(確認の2段階目)。
#[allow(clippy::too_many_arguments)]
fn execute_support_confirm(
    diplo_state: &mut DiplomacyPanelState,
    crisis_registry: &mut CrisisRegistry,
    country_registry: &CountryRegistry,
    supporter: CountryId,
    current_date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    let Some((crisis_id, side)) = diplo_state.pending_support_pledge else {
        return;
    };
    // 確認は必ずここで閉じる(成功/失敗いずれの場合も、確認待ち状態を持ち越さない)。
    diplo_state.pending_support_pledge = None;

    match crisis_response::pledge_support(
        crisis_registry,
        country_registry,
        crisis_id,
        supporter,
        side,
        current_date,
    ) {
        Ok(()) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_support_pledged"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_support_pledge_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

/// P21-013: 第三国による支持撤回(確認なしの単発操作)。
#[allow(clippy::too_many_arguments)]
fn execute_support_withdraw(
    crisis_registry: &mut CrisisRegistry,
    crisis_id: DiplomaticCrisisId,
    supporter: CountryId,
    current_date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    match crisis_response::withdraw_support(crisis_registry, crisis_id, supporter, current_date) {
        Ok(()) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_support_withdrawn"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_support_withdraw_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

/// P21-013: 支持表明コマンド専用のハンドラ。`handle_daily_diplomacy`
/// (`DailySimulationSet::Diplomacy`、AI対象国の日次応答処理)より後に明示的に配置する
/// (`DiplomacyPluginUI::build`の登録を参照)。同一フレームで日付変更と支持操作が
/// 重なった場合、AIの日次応答(期限切れタイムアウトを含む)を必ず先に処理させるため
/// (`DailySimulationSet`全体の列挙順自体は変更しない — この配置は同じ`Update`内の
/// 別プラグインのSystem同士に対する明示的な`.after()`制約であり、
/// `configure_sets`のSystemSet列挙順とは独立)。
///
/// **`.after()`順序だけには依存しない**: セーブロード直後など、`deadline_date`が既に
/// 過去でも`current_phase`がまだ`DemandSent`のまま(`handle_daily_diplomacy`が一度も
/// 走っていない)状態が起こり得るため、`GameDate`を`crisis_response::pledge_support`/
/// `withdraw_support`へ明示的に渡し、ドメインAPI自身(`crisis_is_awaiting_support_response`)
/// にも期限到達を再検証させる。stale化したCrisis(期限切れ・AI応答済み等でphaseが
/// `DemandSent`でなくなったもの、または期限到達済みだがphase未更新のもの)への支持操作は、
/// これにより自動的に拒否される。
#[allow(clippy::too_many_arguments)]
fn handle_crisis_support_command_buttons(
    btn_q: Query<(&Interaction, &CrisisCommandButton), Changed<Interaction>>,
    mut diplo_state: ResMut<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    country_registry: Res<CountryRegistry>,
    date: Res<GameDate>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player) = player_country.0 else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            CrisisCommand::RequestSupportInitiator(crisis_id) => execute_support_request(
                &mut diplo_state,
                crisis_id,
                crisis_response::CrisisSupportSide::Initiator,
            ),
            CrisisCommand::RequestSupportTarget(crisis_id) => execute_support_request(
                &mut diplo_state,
                crisis_id,
                crisis_response::CrisisSupportSide::Target,
            ),
            CrisisCommand::CancelSupport => execute_support_cancel(&mut diplo_state),
            CrisisCommand::ConfirmSupport => execute_support_confirm(
                &mut diplo_state,
                &mut crisis_registry,
                &country_registry,
                player,
                &date,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            CrisisCommand::WithdrawSupport(crisis_id) => execute_support_withdraw(
                &mut crisis_registry,
                crisis_id,
                player,
                &date,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            // P21-010/011のコマンドは`handle_crisis_command_buttons`が処理する。
            CrisisCommand::RequestStart(_)
            | CrisisCommand::Confirm
            | CrisisCommand::Cancel
            | CrisisCommand::RequestAccept(_)
            | CrisisCommand::ConfirmAccept
            | CrisisCommand::CancelAccept
            | CrisisCommand::RequestReject(_)
            | CrisisCommand::ConfirmReject
            | CrisisCommand::CancelReject
            | CrisisCommand::Withdraw(_) => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// P21-011: Crisis受諾/拒否/撤回UIの描画ヘルパー
// ─────────────────────────────────────────────────────────────────────────

/// 単発のCrisisCommandButtonを1つ描画する(`RequestAccept`/`RequestReject`/`Withdraw`用)。
fn spawn_crisis_command_button(
    parent: &mut ChildSpawnerCommands,
    catalog: &TranslationCatalog,
    locale: crate::localization::Locale,
    label_key: &'static str,
    command: CrisisCommand,
    bg_color: Color,
) {
    parent
        .spawn((
            CrisisCommandButton(command),
            Button,
            Node {
                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(bg_color),
        ))
        .with_children(|b| {
            let (text, marker) = localized_text(catalog, locale, label_key, vec![]);
            b.spawn((
                text,
                marker,
                TextColor(Color::WHITE),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
            ));
        });
}

/// 受諾/拒否の2段階確認(プロンプト文 + Confirm/Cancelボタン行)を描画する
/// (既存の`crisis_confirm_prompt`/`Confirm`/`Cancel`パターンと同じ構造)。
#[allow(clippy::too_many_arguments)]
fn spawn_crisis_confirm_row(
    parent: &mut ChildSpawnerCommands,
    catalog: &TranslationCatalog,
    locale: crate::localization::Locale,
    prompt_key: &'static str,
    confirm_label_key: &'static str,
    cancel_label_key: &'static str,
    confirm_command: CrisisCommand,
    cancel_command: CrisisCommand,
    confirm_color: Color,
) {
    let (text, marker) = localized_text(catalog, locale, prompt_key, vec![]);
    parent.spawn((
        text,
        marker,
        TextColor(Color::srgb(1.0, 0.8, 0.3)),
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
    ));
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            spawn_crisis_command_button(
                row,
                catalog,
                locale,
                confirm_label_key,
                confirm_command,
                confirm_color,
            );
            spawn_crisis_command_button(
                row,
                catalog,
                locale,
                cancel_label_key,
                cancel_command,
                Color::srgba(0.4, 0.4, 0.4, 1.0),
            );
        });
}

#[allow(clippy::too_many_arguments)]
fn execute_crisis_confirm(
    diplo_state: &mut DiplomacyPanelState,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &ClaimRegistry,
    player: CountryId,
    target: CountryId,
    state_registry: &StateRegistry,
    date: &GameDate,
    notif_writer: &mut MessageWriter<GameNotification>,
    locale: &CurrentLocale,
    catalog: &TranslationCatalog,
) {
    let Some(claim_id) = diplo_state.pending_crisis_claim else {
        return;
    };
    // 確認は必ずここで閉じる(成功/失敗いずれの場合も、確認待ち状態を持ち越さない)。
    diplo_state.pending_crisis_claim = None;

    let Some(claim) = claim_registry.claims.get(&claim_id) else {
        notif_writer.write(GameNotification {
            message: t(catalog, locale.0, "diplomacy_panel.notif_crisis_claim_gone"),
        });
        return;
    };

    match crisis_registry.start_crisis(claim, player, target, date.display(), state_registry) {
        Ok(_) => {
            notif_writer.write(GameNotification {
                message: t(catalog, locale.0, "diplomacy_panel.notif_crisis_started"),
            });
        }
        Err(err) => {
            notif_writer.write(GameNotification {
                message: tf(
                    catalog,
                    locale.0,
                    "diplomacy_panel.notif_crisis_failed",
                    vec![("reason", t(catalog, locale.0, err))],
                ),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_crisis_command_buttons(
    btn_q: Query<(&Interaction, &CrisisCommandButton), Changed<Interaction>>,
    mut diplo_state: ResMut<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    mut claim_registry: ResMut<ClaimRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    date: Res<GameDate>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(player) = player_country.0 else {
        return;
    };
    let Some(target) = diplo_state.target_country else {
        return;
    };

    for (interaction, btn) in btn_q.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match btn.0 {
            CrisisCommand::RequestStart(claim_id) => {
                execute_crisis_request_start(&mut diplo_state, claim_id)
            }
            CrisisCommand::Cancel => execute_crisis_cancel(&mut diplo_state),
            CrisisCommand::Confirm => execute_crisis_confirm(
                &mut diplo_state,
                &mut crisis_registry,
                &claim_registry,
                player,
                target,
                &state_registry,
                &date,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            CrisisCommand::RequestAccept(crisis_id) => {
                execute_crisis_request_accept(&mut diplo_state, crisis_id)
            }
            CrisisCommand::CancelAccept => execute_crisis_cancel_accept(&mut diplo_state),
            CrisisCommand::ConfirmAccept => execute_crisis_confirm_accept(
                &mut diplo_state,
                &mut crisis_registry,
                &mut claim_registry,
                &mut state_registry,
                player,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            CrisisCommand::RequestReject(crisis_id) => {
                execute_crisis_request_reject(&mut diplo_state, crisis_id)
            }
            CrisisCommand::CancelReject => execute_crisis_cancel_reject(&mut diplo_state),
            CrisisCommand::ConfirmReject => execute_crisis_confirm_reject(
                &mut diplo_state,
                &mut crisis_registry,
                &mut justification_registry,
                player,
                &date,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            CrisisCommand::Withdraw(crisis_id) => execute_crisis_withdraw(
                &mut crisis_registry,
                &mut justification_registry,
                crisis_id,
                player,
                &mut notif_writer,
                &locale,
                &catalog,
            ),
            // P21-013: 支持関連コマンドは`handle_crisis_support_command_buttons`
            // (`handle_daily_diplomacy`の後に明示的に配置)が処理する。ここでは無視する。
            CrisisCommand::RequestSupportInitiator(_)
            | CrisisCommand::RequestSupportTarget(_)
            | CrisisCommand::ConfirmSupport
            | CrisisCommand::CancelSupport
            | CrisisCommand::WithdrawSupport(_) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_diplomacy_panel_ui(
    mut commands: Commands,
    state: Res<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    diplo_registry: Res<DiplomacyRegistry>,
    state_registry: Res<StateRegistry>,
    justification_registry: Res<WarJustificationRegistry>,
    war_registry: Res<WarRegistry>,
    claim_registry: Res<ClaimRegistry>,
    crisis_registry: Res<CrisisRegistry>,
    date: Res<GameDate>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut header_q: Query<(&mut Text, &mut LocalizedText), With<DiplomacyHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<DiplomacyContentContainer>>,
) {
    if !state.open && !locale.is_changed() {
        return;
    }

    let Ok((mut header_text, mut header_marker)) = header_q.single_mut() else {
        return;
    };
    let Ok((container_entity, children_opt)) = container_q.single() else {
        return;
    };

    let Some(p_cid) = player_country.0 else {
        return;
    };
    let Some(target_cid) = state.target_country else {
        let key = "diplomacy_panel.select_prompt";
        let rendered = t(&catalog, locale.0, key);
        if header_text.0 != rendered {
            *header_text = Text::new(rendered);
        }
        header_marker.key = key;
        header_marker.args = vec![];
        if let Some(children) = children_opt {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        return;
    };

    if target_cid == p_cid {
        let key = "diplomacy_panel.self_selected";
        let rendered = t(&catalog, locale.0, key);
        if header_text.0 != rendered {
            *header_text = Text::new(rendered);
        }
        header_marker.key = key;
        header_marker.args = vec![];
        if let Some(children) = children_opt {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        return;
    }

    let Some(proposer) = country_registry.get(p_cid) else {
        return;
    };
    let Some(target) = country_registry.get(target_cid) else {
        return;
    };
    let rel = diplo_registry.get_or_default(p_cid, target_cid);

    let header_args = vec![
        ("country", target.name.clone()),
        ("id", target.id.0.to_string()),
        ("opinion", format!("{:+.1}", rel.opinion)),
    ];
    let header_info = tf(
        &catalog,
        locale.0,
        "diplomacy_panel.header",
        header_args.clone(),
    );
    if header_text.0 != header_info {
        *header_text = Text::new(header_info);
    }
    header_marker.key = "diplomacy_panel.header";
    header_marker.args = header_args;

    if let Some(children) = children_opt {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(container_entity).with_children(|parent| {
        parent
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(6.0)),
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            })
            .insert(BackgroundColor(Color::srgba(0.12, 0.15, 0.18, 0.9)))
            .with_children(|col| {
                if let Some(ref act) = rel.active_activity {
                    let side_key = if act.initiator == p_cid {
                        "diplomacy_panel.activity_initiated_by_you"
                    } else {
                        "diplomacy_panel.activity_initiated_by_foreign"
                    };
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.active_activity",
                        vec![
                            (
                                "activity",
                                t(&catalog, locale.0, act.activity_type.display_name()),
                            ),
                            ("days", act.days_remaining.to_string()),
                            ("side", t(&catalog, locale.0, side_key)),
                        ],
                    );
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.9, 0.9, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                } else {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.no_active_activity",
                        vec![],
                    );
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                }

                if let Some(&cd) = rel.cooldowns.get(&p_cid) {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.cooldown_active",
                        vec![("days", cd.to_string())],
                    );
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(1.0, 0.6, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                }
            });

        parent
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                padding: UiRect::all(Val::Px(6.0)),
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            })
            .insert(BackgroundColor(Color::srgba(0.12, 0.15, 0.18, 0.9)))
            .with_children(|col| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.treaties_header",
                    vec![],
                );
                col.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.6, 0.9, 0.7)),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                ));

                let active_treaties: Vec<_> = rel.treaties.iter().filter(|t| t.is_active).collect();
                if active_treaties.is_empty() {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "diplomacy_panel.treaties_none", vec![]);
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    for treaty in active_treaties {
                        col.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "diplomacy_panel.treaty_line",
                                vec![
                                    (
                                        "treaty",
                                        t(&catalog, locale.0, treaty.treaty_type.display_name()),
                                    ),
                                    ("date", treaty.signed_date.clone()),
                                ],
                            );
                            row.spawn((
                                text,
                                marker,
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            row.spawn((
                                BreakTreatyButton(target_cid, treaty.treaty_type),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.6, 0.2, 0.2, 1.0)),
                            ))
                            .with_children(|b| {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "diplomacy_panel.break_button",
                                    vec![],
                                );
                                b.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            });
                        });
                    }
                }
            });

        let cd_active = rel.cooldowns.contains_key(&p_cid);
        let act_active = rel.active_activity.is_some();
        let can_start_activity = !cd_active && !act_active;

        parent
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                margin: UiRect::bottom(Val::Px(4.0)),
                ..default()
            })
            .with_children(|row| {
                if can_start_activity {
                    row.spawn((
                        ImproveRelationsButton(target_cid),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.2, 0.5, 0.3, 1.0)),
                    ))
                    .with_children(|b| {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.improve_relations_button",
                            vec![],
                        );
                        b.spawn((
                            text,
                            marker,
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });

                    row.spawn((
                        HarmRelationsButton(target_cid),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.5, 0.2, 0.2, 1.0)),
                    ))
                    .with_children(|b| {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.harm_relations_button",
                            vec![],
                        );
                        b.spawn((
                            text,
                            marker,
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
                } else {
                    let reason_key = if act_active {
                        "diplomacy_panel.reason_activity_in_progress"
                    } else {
                        "diplomacy_panel.reason_cooldown_active"
                    };
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.actions_disabled",
                        vec![("reason", t(&catalog, locale.0, reason_key))],
                    );
                    row.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                }
            });

        let (text, marker) =
            localized_text(&catalog, locale.0, "diplomacy_panel.propose_header", vec![]);
        parent.spawn((
            text,
            marker,
            TextColor(Color::srgb(0.9, 0.85, 0.6)),
            TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            },
        ));

        for &proposal in &[TreatyType::NonAggressionPact, TreatyType::Alliance] {
            let already_signed = rel.has_treaty(proposal);
            let breakdown =
                calculate_proposal_score(proposal, proposer, target, &rel, &state_registry);

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .insert(BackgroundColor(Color::srgba(0.12, 0.12, 0.16, 0.9)))
                .with_children(|col| {
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        let result_key = if already_signed {
                            "diplomacy_panel.result_already_signed"
                        } else if breakdown.accepted {
                            "diplomacy_panel.result_accept"
                        } else {
                            "diplomacy_panel.result_reject"
                        };
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.propose_line",
                            vec![
                                ("treaty", t(&catalog, locale.0, proposal.display_name())),
                                ("total", format!("{:.1}", breakdown.total_score)),
                                ("required", format!("{:.1}", breakdown.required_score)),
                                ("result", t(&catalog, locale.0, result_key)),
                            ],
                        );
                        row.spawn((
                            text,
                            marker,
                            TextColor(if already_signed {
                                Color::srgb(0.7, 0.7, 0.7)
                            } else if breakdown.accepted {
                                Color::srgb(0.6, 0.9, 0.6)
                            } else {
                                Color::srgb(0.9, 0.6, 0.6)
                            }),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));

                        if !already_signed {
                            row.spawn((
                                ProposeTreatyButton(target_cid, proposal),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(if breakdown.accepted {
                                    Color::srgba(0.2, 0.5, 0.3, 1.0)
                                } else {
                                    Color::srgba(0.4, 0.4, 0.4, 1.0)
                                }),
                            ))
                            .with_children(|b| {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "diplomacy_panel.propose_button",
                                    vec![],
                                );
                                b.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            });
                        }
                    });

                    let item_str = breakdown
                        .items
                        .iter()
                        .map(|i| format!("{}: {:+.1}", i.label, i.score))
                        .collect::<Vec<_>>()
                        .join(" | ");

                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.breakdown_line",
                        vec![("items", item_str)],
                    );
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(10.0),
                            ..default()
                        },
                    ));
                });
        }

        // ── 5. 戦争正当化 & 宣戦布告セクション ───────────────────────
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.12, 0.05, 0.05, 0.9)),
            ))
            .with_children(|sec| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.war_section_title",
                    vec![],
                );
                sec.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.9, 0.4, 0.4)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                ));

                let has_alliance = rel.has_treaty(TreatyType::Alliance);
                let has_nap = rel.has_treaty(TreatyType::NonAggressionPact);
                let is_already_war = war_registry.are_countries_at_war(p_cid, target_cid);

                if is_already_war {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.currently_at_war",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(1.0, 0.3, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else if has_alliance {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.alliance_blocks_war",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.9, 0.6, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else if has_nap {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.nap_blocks_war",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.9, 0.6, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    // 対象国家が所有する陸上州一覧を取得
                    let owned_states: Vec<_> = state_registry
                        .states
                        .iter()
                        .filter(|s| s.owner_country_id == target_cid)
                        .collect();

                    if owned_states.is_empty() {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.target_no_states",
                            vec![],
                        );
                        sec.spawn((
                            text,
                            marker,
                            TextColor(Color::srgb(0.7, 0.7, 0.7)),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    } else {
                        for st in owned_states {
                            let ready_just = justification_registry
                                .get_ready_justification(p_cid, target_cid, st.id);

                            let active_just =
                                justification_registry.justifications.values().find(|j| {
                                    j.initiator == p_cid
                                        && j.target == target_cid
                                        && j.target_state == st.id
                                });

                            sec.spawn((Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(8.0),
                                ..default()
                            },))
                                .with_children(|row| {
                                    if ready_just.is_some() {
                                        let (text, marker) = localized_text(
                                            &catalog,
                                            locale.0,
                                            "diplomacy_panel.goal_ready",
                                            vec![("state", st.name.clone())],
                                        );
                                        row.spawn((
                                            text,
                                            marker,
                                            TextColor(Color::srgb(0.4, 0.9, 0.4)),
                                            TextFont {
                                                font_size: FontSize::Px(11.0),
                                                ..default()
                                            },
                                        ));

                                        row.spawn((
                                            DeclareWarButton(target_cid, st.id),
                                            Button,
                                            Node {
                                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.8, 0.1, 0.1, 1.0)),
                                        ))
                                        .with_children(
                                            |b| {
                                                let (text, marker) = localized_text(
                                                    &catalog,
                                                    locale.0,
                                                    "diplomacy_panel.declare_war_button",
                                                    vec![],
                                                );
                                                b.spawn((
                                                    text,
                                                    marker,
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            },
                                        );
                                    } else if let Some(j) = active_just {
                                        let (text, marker) = localized_text(
                                            &catalog,
                                            locale.0,
                                            "diplomacy_panel.justifying",
                                            vec![
                                                ("state", st.name.clone()),
                                                ("days", j.days_passed.to_string()),
                                                ("required", j.required_days.to_string()),
                                            ],
                                        );
                                        row.spawn((
                                            text,
                                            marker,
                                            TextColor(Color::srgb(0.9, 0.8, 0.3)),
                                            TextFont {
                                                font_size: FontSize::Px(11.0),
                                                ..default()
                                            },
                                        ));
                                    } else {
                                        let (text, marker) = localized_text(
                                            &catalog,
                                            locale.0,
                                            "diplomacy_panel.target_state_line",
                                            vec![("state", st.name.clone())],
                                        );
                                        row.spawn((
                                            text,
                                            marker,
                                            TextColor(Color::srgb(0.8, 0.8, 0.8)),
                                            TextFont {
                                                font_size: FontSize::Px(11.0),
                                                ..default()
                                            },
                                        ));

                                        row.spawn((
                                            JustifyWarButton(target_cid, st.id),
                                            Button,
                                            Node {
                                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.5, 0.2, 0.2, 1.0)),
                                        ))
                                        .with_children(
                                            |b| {
                                                let (text, marker) = localized_text(
                                                    &catalog,
                                                    locale.0,
                                                    "diplomacy_panel.justify_war_button",
                                                    vec![],
                                                );
                                                b.spawn((
                                                    text,
                                                    marker,
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            },
                                        );
                                    }
                                });
                        }
                    }
                }
            });

        // ── P21-010: 領土請求(Claim)セクション ────────────────────
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.1, 0.12, 0.9)),
            ))
            .with_children(|sec| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.claim_section_title",
                    vec![],
                );
                sec.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.5, 0.8, 0.9)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                ));

                let claimable_states: Vec<_> = state_registry
                    .states
                    .iter()
                    .filter(|s| s.owner_country_id == target_cid && !s.is_sea)
                    .collect();

                if claimable_states.is_empty() {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.claim_no_states",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    for st in &claimable_states {
                        let is_selected = state.claim_target_state == Some(st.id);
                        sec.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            let key = if is_selected {
                                "diplomacy_panel.claim_state_selected"
                            } else {
                                "diplomacy_panel.claim_state_line"
                            };
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                key,
                                vec![("state", st.name.clone())],
                            );
                            row.spawn((
                                text,
                                marker,
                                TextColor(if is_selected {
                                    Color::srgb(0.6, 0.9, 0.9)
                                } else {
                                    Color::srgb(0.8, 0.8, 0.8)
                                }),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            if !is_selected {
                                row.spawn((
                                    ClaimCommandButton(ClaimCommand::SelectState(st.id)),
                                    Button,
                                    Node {
                                        padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.2, 0.4, 0.45, 1.0)),
                                ))
                                .with_children(|b| {
                                    let (text, marker) = localized_text(
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.claim_select_button",
                                        vec![],
                                    );
                                    b.spawn((
                                        text,
                                        marker,
                                        TextColor(Color::WHITE),
                                        TextFont {
                                            font_size: FontSize::Px(10.0),
                                            ..default()
                                        },
                                    ));
                                });
                            }
                        });
                    }

                    if let Some(sel_id) = state.claim_target_state {
                        match claim_registry.can_create_claim(
                            p_cid,
                            target_cid,
                            sel_id,
                            &country_registry,
                            &state_registry,
                        ) {
                            Ok(()) => {
                                sec.spawn((
                                    ClaimCommandButton(ClaimCommand::Create),
                                    Button,
                                    Node {
                                        padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                        margin: UiRect::top(Val::Px(4.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.2, 0.5, 0.55, 1.0)),
                                ))
                                .with_children(|b| {
                                    let (text, marker) = localized_text(
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.claim_create_button",
                                        vec![],
                                    );
                                    b.spawn((
                                        text,
                                        marker,
                                        TextColor(Color::WHITE),
                                        TextFont {
                                            font_size: FontSize::Px(11.0),
                                            ..default()
                                        },
                                    ));
                                });
                            }
                            Err(reason) => {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "diplomacy_panel.claim_create_disabled",
                                    vec![("reason", t(&catalog, locale.0, reason))],
                                );
                                sec.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::srgb(0.9, 0.6, 0.4)),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            }
                        }
                    }
                }
            });

        // ── P21-010: 自国Claim一覧 + 危機開始セクション ────────────
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.08, 0.05, 0.9)),
            ))
            .with_children(|sec| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.my_claims_header",
                    vec![],
                );
                sec.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.9, 0.7, 0.5)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                ));

                let my_claims: Vec<_> = claim_registry
                    .get_claims_by_country(p_cid)
                    .into_iter()
                    .filter(|c| {
                        state_registry
                            .get(c.target_state)
                            .is_some_and(|s| s.owner_country_id == target_cid)
                    })
                    .collect();

                if my_claims.is_empty() {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.my_claims_none",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    for claim in my_claims {
                        let st_name = state_registry
                            .get(claim.target_state)
                            .map(|s| s.name.clone())
                            .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));

                        sec.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(2.0),
                            margin: UiRect::bottom(Val::Px(4.0)),
                            ..default()
                        })
                        .with_children(|claim_col| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "diplomacy_panel.claim_line",
                                vec![
                                    ("state", st_name),
                                    ("source", format!("{:?}", claim.source)),
                                    ("date", claim.created_date.clone()),
                                ],
                            );
                            claim_col.spawn((
                                text,
                                marker,
                                TextColor(Color::srgb(0.9, 0.85, 0.7)),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            if state.pending_crisis_claim == Some(claim.id) {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "diplomacy_panel.crisis_confirm_prompt",
                                    vec![],
                                );
                                claim_col.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::srgb(1.0, 0.8, 0.3)),
                                    TextFont {
                                        font_size: FontSize::Px(11.0),
                                        ..default()
                                    },
                                ));
                                claim_col
                                    .spawn(Node {
                                        flex_direction: FlexDirection::Row,
                                        column_gap: Val::Px(8.0),
                                        ..default()
                                    })
                                    .with_children(|row| {
                                        row.spawn((
                                            CrisisCommandButton(CrisisCommand::Confirm),
                                            Button,
                                            Node {
                                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.7, 0.15, 0.1, 1.0)),
                                        ))
                                        .with_children(
                                            |b| {
                                                let (text, marker) = localized_text(
                                                    &catalog,
                                                    locale.0,
                                                    "diplomacy_panel.crisis_confirm_button",
                                                    vec![],
                                                );
                                                b.spawn((
                                                    text,
                                                    marker,
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            },
                                        );

                                        row.spawn((
                                            CrisisCommandButton(CrisisCommand::Cancel),
                                            Button,
                                            Node {
                                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.4, 0.4, 0.4, 1.0)),
                                        ))
                                        .with_children(
                                            |b| {
                                                let (text, marker) = localized_text(
                                                    &catalog,
                                                    locale.0,
                                                    "diplomacy_panel.crisis_cancel_button",
                                                    vec![],
                                                );
                                                b.spawn((
                                                    text,
                                                    marker,
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            },
                                        );
                                    });
                            } else {
                                match crisis_registry.can_start_crisis(
                                    claim,
                                    p_cid,
                                    target_cid,
                                    &state_registry,
                                ) {
                                    Ok(()) => {
                                        claim_col
                                            .spawn((
                                                CrisisCommandButton(CrisisCommand::RequestStart(
                                                    claim.id,
                                                )),
                                                Button,
                                                Node {
                                                    padding: UiRect::axes(
                                                        Val::Px(8.0),
                                                        Val::Px(3.0),
                                                    ),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.6, 0.2, 0.1, 1.0)),
                                            ))
                                            .with_children(|b| {
                                                let (text, marker) = localized_text(
                                                    &catalog,
                                                    locale.0,
                                                    "diplomacy_panel.start_crisis_button",
                                                    vec![],
                                                );
                                                b.spawn((
                                                    text,
                                                    marker,
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            });
                                    }
                                    Err(reason) => {
                                        let (text, marker) = localized_text(
                                            &catalog,
                                            locale.0,
                                            "diplomacy_panel.crisis_start_disabled",
                                            vec![("reason", t(&catalog, locale.0, reason))],
                                        );
                                        claim_col.spawn((
                                            text,
                                            marker,
                                            TextColor(Color::srgb(0.8, 0.6, 0.5)),
                                            TextFont {
                                                font_size: FontSize::Px(10.0),
                                                ..default()
                                            },
                                        ));
                                    }
                                }
                            }
                        });
                    }
                }
            });

        // ── P21-010: 進行中外交危機一覧セクション ──────────────────
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.05, 0.1, 0.9)),
            ))
            .with_children(|sec| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.active_crises_header",
                    vec![],
                );
                sec.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.9, 0.5, 0.8)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                ));

                // P21-013: 第三国が支持表明できるよう、当事国(initiator/target)に
                // 限らず全Crisisを表示する(P21-010/011は`p_cid`が当事国のものだけに
                // 限定していた)。
                let mut visible_crises: Vec<_> = crisis_registry.crises.values().collect();
                visible_crises.sort_by_key(|c| c.id.0);

                if visible_crises.is_empty() {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.no_active_crises",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.6, 0.6, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    for crisis in visible_crises {
                        let country_name = |cid: CountryId| {
                            country_registry
                                .get(cid)
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"))
                        };
                        let is_party = crisis.initiator == p_cid || crisis.target == p_cid;
                        let other_name = if crisis.initiator == p_cid {
                            country_name(crisis.target)
                        } else if crisis.target == p_cid {
                            country_name(crisis.initiator)
                        } else {
                            // P21-013: 第三国視点では「相手国」が定まらないため、
                            // 両当事国を併記する。
                            format!(
                                "{} / {}",
                                country_name(crisis.initiator),
                                country_name(crisis.target)
                            )
                        };
                        let target_states = crisis
                            .war_goals
                            .first()
                            .map(|g| {
                                g.target_states
                                    .iter()
                                    .filter_map(|sid| {
                                        state_registry.get(*sid).map(|s| s.name.clone())
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let phase_key = match crisis.current_phase {
                            CrisisPhase::Preparing => "diplomacy_panel.crisis_phase_preparing",
                            CrisisPhase::DemandSent => "diplomacy_panel.crisis_phase_demand_sent",
                            CrisisPhase::Negotiating => "diplomacy_panel.crisis_phase_negotiating",
                            CrisisPhase::Escalating => "diplomacy_panel.crisis_phase_escalating",
                            CrisisPhase::ResolvedPeacefully => {
                                "diplomacy_panel.crisis_phase_resolved"
                            }
                            CrisisPhase::WarStarted => "diplomacy_panel.crisis_phase_war_started",
                            CrisisPhase::Cancelled => "diplomacy_panel.crisis_phase_cancelled",
                        };

                        sec.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(2.0),
                            margin: UiRect::bottom(Val::Px(4.0)),
                            ..default()
                        })
                        .with_children(|crisis_col| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "diplomacy_panel.crisis_line",
                                vec![
                                    ("country", other_name),
                                    ("states", target_states),
                                    ("phase", t(&catalog, locale.0, phase_key)),
                                    ("days", crisis.days_in_phase.to_string()),
                                ],
                            );
                            crisis_col.spawn((
                                text,
                                marker,
                                TextColor(Color::srgb(0.9, 0.7, 0.9)),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            // P21-011: 要求送付済み(DemandSent)の間だけ期限を表示する。
                            if crisis.current_phase == CrisisPhase::DemandSent
                                && let Some(deadline) = &crisis.deadline_date
                            {
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "diplomacy_panel.crisis_deadline_line",
                                    vec![("date", deadline.clone())],
                                );
                                crisis_col.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::srgb(1.0, 0.8, 0.4)),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            }

                            // P21-013: 支持国一覧(全プレイヤーへ常時表示、terminal
                            // Crisisでも履歴として表示する)。
                            let mut initiator_supporters: Vec<String> = Vec::new();
                            let mut target_supporters: Vec<String> = Vec::new();
                            let mut my_support_side: Option<ThirdCountryReaction> = None;
                            let mut supporters: Vec<_> =
                                crisis.third_party_reactions.iter().collect();
                            supporters.sort_by_key(|(cid, _)| cid.0);
                            for (&supporter_id, reaction) in supporters {
                                match reaction {
                                    ThirdCountryReaction::SupportsInitiator => {
                                        initiator_supporters.push(country_name(supporter_id));
                                    }
                                    ThirdCountryReaction::SupportsTarget => {
                                        target_supporters.push(country_name(supporter_id));
                                    }
                                    ThirdCountryReaction::Neutral
                                    | ThirdCountryReaction::CondemnsInitiator => {}
                                }
                                if supporter_id == p_cid {
                                    my_support_side = Some(*reaction);
                                }
                            }
                            let none_label =
                                t(&catalog, locale.0, "diplomacy_panel.crisis_support_none");
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "diplomacy_panel.crisis_support_initiator_line",
                                vec![(
                                    "countries",
                                    if initiator_supporters.is_empty() {
                                        none_label.clone()
                                    } else {
                                        initiator_supporters.join(", ")
                                    },
                                )],
                            );
                            crisis_col.spawn((
                                text,
                                marker,
                                TextColor(Color::srgb(0.7, 0.85, 1.0)),
                                TextFont {
                                    font_size: FontSize::Px(10.0),
                                    ..default()
                                },
                            ));
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "diplomacy_panel.crisis_support_target_line",
                                vec![(
                                    "countries",
                                    if target_supporters.is_empty() {
                                        none_label
                                    } else {
                                        target_supporters.join(", ")
                                    },
                                )],
                            );
                            crisis_col.spawn((
                                text,
                                marker,
                                TextColor(Color::srgb(1.0, 0.75, 0.7)),
                                TextFont {
                                    font_size: FontSize::Px(10.0),
                                    ..default()
                                },
                            ));
                            if let Some(side) = my_support_side {
                                let key = match side {
                                    ThirdCountryReaction::SupportsInitiator => {
                                        "diplomacy_panel.crisis_your_support_initiator_line"
                                    }
                                    _ => "diplomacy_panel.crisis_your_support_target_line",
                                };
                                let (text, marker) =
                                    localized_text(&catalog, locale.0, key, vec![]);
                                crisis_col.spawn((
                                    text,
                                    marker,
                                    TextColor(Color::srgb(1.0, 0.9, 0.5)),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            }

                            // P21-013修正: 第三国(initiatorでもtargetでもない)視点 --
                            // 支持ボタン/確認/撤回。ドメインAPI(`can_pledge_support`/
                            // `can_withdraw_support`)と全く同じ判定関数
                            // (`crisis_is_awaiting_support_response`)を使い、期限到達済み
                            // だがphaseがまだDemandSentのまま(ロード直後等)のケースでも
                            // ボタン自体を出さないようにする(判定基準が2箇所に分かれて
                            // ずれることを防ぐ)。
                            if !is_party
                                && crisis_response::crisis_is_awaiting_support_response(
                                    crisis, &date,
                                )
                            {
                                if state.pending_support_pledge
                                    == Some((
                                        crisis.id,
                                        crisis_response::CrisisSupportSide::Initiator,
                                    ))
                                {
                                    spawn_crisis_confirm_row(
                                        crisis_col,
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_support_initiator_confirm_prompt",
                                        "diplomacy_panel.crisis_support_confirm_button",
                                        "diplomacy_panel.crisis_cancel_button",
                                        CrisisCommand::ConfirmSupport,
                                        CrisisCommand::CancelSupport,
                                        Color::srgba(0.15, 0.4, 0.6, 1.0),
                                    );
                                } else if state.pending_support_pledge
                                    == Some((crisis.id, crisis_response::CrisisSupportSide::Target))
                                {
                                    spawn_crisis_confirm_row(
                                        crisis_col,
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_support_target_confirm_prompt",
                                        "diplomacy_panel.crisis_support_confirm_button",
                                        "diplomacy_panel.crisis_cancel_button",
                                        CrisisCommand::ConfirmSupport,
                                        CrisisCommand::CancelSupport,
                                        Color::srgba(0.6, 0.25, 0.15, 1.0),
                                    );
                                } else if my_support_side.is_some() {
                                    spawn_crisis_command_button(
                                        crisis_col,
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_withdraw_support_button",
                                        CrisisCommand::WithdrawSupport(crisis.id),
                                        Color::srgba(0.4, 0.4, 0.4, 1.0),
                                    );
                                } else {
                                    crisis_col
                                        .spawn(Node {
                                            flex_direction: FlexDirection::Row,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            spawn_crisis_command_button(
                                                row,
                                                &catalog,
                                                locale.0,
                                                "diplomacy_panel.crisis_support_initiator_button",
                                                CrisisCommand::RequestSupportInitiator(crisis.id),
                                                Color::srgba(0.15, 0.35, 0.55, 1.0),
                                            );
                                            spawn_crisis_command_button(
                                                row,
                                                &catalog,
                                                locale.0,
                                                "diplomacy_panel.crisis_support_target_button",
                                                CrisisCommand::RequestSupportTarget(crisis.id),
                                                Color::srgba(0.55, 0.25, 0.15, 1.0),
                                            );
                                        });
                                }
                            }

                            // P21-011: 対象国(target)視点 -- 受諾/拒否ボタン。
                            if crisis.target == p_cid
                                && crisis.current_phase == CrisisPhase::DemandSent
                            {
                                if state.pending_crisis_accept == Some(crisis.id) {
                                    spawn_crisis_confirm_row(
                                        crisis_col,
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_accept_confirm_prompt",
                                        "diplomacy_panel.crisis_accept_confirm_button",
                                        "diplomacy_panel.crisis_cancel_button",
                                        CrisisCommand::ConfirmAccept,
                                        CrisisCommand::CancelAccept,
                                        Color::srgba(0.15, 0.55, 0.2, 1.0),
                                    );
                                } else if state.pending_crisis_reject == Some(crisis.id) {
                                    spawn_crisis_confirm_row(
                                        crisis_col,
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_reject_confirm_prompt",
                                        "diplomacy_panel.crisis_reject_confirm_button",
                                        "diplomacy_panel.crisis_cancel_button",
                                        CrisisCommand::ConfirmReject,
                                        CrisisCommand::CancelReject,
                                        Color::srgba(0.7, 0.15, 0.1, 1.0),
                                    );
                                } else {
                                    crisis_col
                                        .spawn(Node {
                                            flex_direction: FlexDirection::Row,
                                            column_gap: Val::Px(8.0),
                                            ..default()
                                        })
                                        .with_children(|row| {
                                            spawn_crisis_command_button(
                                                row,
                                                &catalog,
                                                locale.0,
                                                "diplomacy_panel.crisis_accept_button",
                                                CrisisCommand::RequestAccept(crisis.id),
                                                Color::srgba(0.15, 0.45, 0.2, 1.0),
                                            );
                                            spawn_crisis_command_button(
                                                row,
                                                &catalog,
                                                locale.0,
                                                "diplomacy_panel.crisis_reject_button",
                                                CrisisCommand::RequestReject(crisis.id),
                                                Color::srgba(0.6, 0.2, 0.1, 1.0),
                                            );
                                        });
                                }
                            }

                            // P21-011: 開始国(initiator)視点 -- 撤回ボタン・正当化取得の表示。
                            if crisis.initiator == p_cid {
                                if matches!(
                                    crisis.current_phase,
                                    CrisisPhase::DemandSent | CrisisPhase::Escalating
                                ) {
                                    crisis_col
                                        .spawn((
                                            CrisisCommandButton(CrisisCommand::Withdraw(crisis.id)),
                                            Button,
                                            Node {
                                                padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.4, 0.4, 0.4, 1.0)),
                                        ))
                                        .with_children(|b| {
                                            let (text, marker) = localized_text(
                                                &catalog,
                                                locale.0,
                                                "diplomacy_panel.crisis_withdraw_button",
                                                vec![],
                                            );
                                            b.spawn((
                                                text,
                                                marker,
                                                TextColor(Color::WHITE),
                                                TextFont {
                                                    font_size: FontSize::Px(10.0),
                                                    ..default()
                                                },
                                            ));
                                        });
                                }
                                if crisis.current_phase == CrisisPhase::Escalating
                                    && crisis.related_justification_id.is_some()
                                {
                                    let (text, marker) = localized_text(
                                        &catalog,
                                        locale.0,
                                        "diplomacy_panel.crisis_justification_ready_line",
                                        vec![],
                                    );
                                    crisis_col.spawn((
                                        text,
                                        marker,
                                        TextColor(Color::srgb(1.0, 0.7, 0.3)),
                                        TextFont {
                                            font_size: FontSize::Px(10.0),
                                            ..default()
                                        },
                                    ));
                                }
                            }
                        });
                    }
                }
            });

        // ── 6. 進行中戦争一覧セクション ────────────────────────────
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(12.0)),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.08, 0.08, 0.12, 0.9)),
            ))
            .with_children(|sec| {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "diplomacy_panel.active_wars_header",
                    vec![],
                );
                sec.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.5, 0.7, 0.9)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                ));

                let active_wars: Vec<_> = war_registry
                    .wars
                    .values()
                    .filter(|w| w.status == crate::war::data::WarStatus::Active)
                    .collect();

                if active_wars.is_empty() {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "diplomacy_panel.no_active_wars",
                        vec![],
                    );
                    sec.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.6, 0.6, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    let leader_suffix = t(&catalog, locale.0, "diplomacy_panel.war_leader_suffix");
                    for war in active_wars {
                        // P21-016: 複数参加国を`CountryId`昇順(決定的)で列挙し、代表国
                        // (primary_attacker/primary_defender)は色ではなくテキストの
                        // 接尾辞で明示する(色覚多様性に配慮)。
                        let primary_attacker = war.primary_attacker_id();
                        let primary_defender = war.primary_defender_id();
                        let attacker_names: Vec<String> = war
                            .sorted_attackers()
                            .into_iter()
                            .filter_map(|cid| {
                                country_registry.get(cid).map(|c| {
                                    if cid == primary_attacker {
                                        format!("{}{}", c.name, leader_suffix)
                                    } else {
                                        c.name.clone()
                                    }
                                })
                            })
                            .collect();
                        let defender_names: Vec<String> = war
                            .sorted_defenders()
                            .into_iter()
                            .filter_map(|cid| {
                                country_registry.get(cid).map(|c| {
                                    if cid == primary_defender {
                                        format!("{}{}", c.name, leader_suffix)
                                    } else {
                                        c.name.clone()
                                    }
                                })
                            })
                            .collect();

                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.war_line",
                            vec![
                                ("id", format!("{:?}", war.id.0)),
                                ("name", war.name.clone()),
                                ("attackers", attacker_names.join(", ")),
                                ("defenders", defender_names.join(", ")),
                                ("date", war.start_date.clone()),
                            ],
                        );
                        sec.spawn((
                            text,
                            marker,
                            TextColor(Color::srgb(0.9, 0.5, 0.5)),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    }
                }
            });
    });
}

/// P21-014: NaN・無限大・負値を絶対に表示しない小数1桁フォーマット。
/// `evaluate_country_power`自体が既に非負・有限を保証しているが、UI表示の防御線として
/// 二重に保証する。
fn format_power_value(value: f32) -> String {
    if value.is_finite() {
        format!("{:.1}", value.max(0.0))
    } else {
        "-".to_string()
    }
}

/// P21-014: 1国家分の国家ランク・世界順位・国家総合力・内訳(軍事力/経済力/人口)を
/// 描画する。評価がまだ一度も構築されていない国家(`CountryPowerRegistry`に
/// エントリが無い、`MainMenu`/`CountrySelection`直後など)は安全に「評価中」表示に
/// フォールバックする。存在しない`CountryId`が渡された場合は何も描画しない
/// (panicしない)。
#[allow(clippy::too_many_arguments)]
fn spawn_country_power_block(
    parent: &mut ChildSpawnerCommands,
    catalog: &TranslationCatalog,
    locale: crate::localization::Locale,
    country_registry: &CountryRegistry,
    power_registry: &CountryPowerRegistry,
    country_id: CountryId,
    heading_key: &'static str,
) {
    let Some(country) = country_registry.get(country_id) else {
        return;
    };

    let (heading_text, heading_marker) = localized_text(
        catalog,
        locale,
        heading_key,
        vec![("country", country.name.clone())],
    );
    parent.spawn((
        heading_text,
        heading_marker,
        TextColor(Color::srgb(0.75, 0.85, 0.95)),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
    ));

    let Some(assessment) = power_registry.get(country_id) else {
        let (text, marker) =
            localized_text(catalog, locale, "diplomacy_panel.power_pending", vec![]);
        parent.spawn((
            text,
            marker,
            TextColor(Color::srgb(0.6, 0.6, 0.6)),
            TextFont {
                font_size: FontSize::Px(11.0),
                ..default()
            },
        ));
        return;
    };

    let tier_name = t(catalog, locale, assessment.power_tier.display_name());
    let total_count = power_registry.country_count();

    let lines: [(&'static str, Vec<(&'static str, String)>); 6] = [
        ("diplomacy_panel.power_tier_line", vec![("tier", tier_name)]),
        (
            "diplomacy_panel.power_rank_line",
            vec![
                ("rank", assessment.world_rank.to_string()),
                ("total", total_count.to_string()),
            ],
        ),
        (
            "diplomacy_panel.power_total_line",
            vec![("score", format_power_value(assessment.total_score))],
        ),
        (
            "diplomacy_panel.power_military_line",
            vec![("value", format_power_value(assessment.military_normalized))],
        ),
        (
            "diplomacy_panel.power_economic_line",
            vec![("value", format_power_value(assessment.economic_normalized))],
        ),
        (
            "diplomacy_panel.power_population_line",
            vec![(
                "value",
                format_power_value(assessment.population_normalized),
            )],
        ),
    ];

    for (key, args) in lines {
        let (text, marker) = localized_text(catalog, locale, key, args);
        parent.spawn((
            text,
            marker,
            TextColor(Color::srgb(0.85, 0.85, 0.8)),
            TextFont {
                font_size: FontSize::Px(11.0),
                ..default()
            },
        ));
    }
}

/// P21-014: プレイヤー自国の国家評価を常に表示し、外国が選択されていれば
/// (`DiplomacyPanelState::target_country`)その国の評価も追加で表示する。
/// `DiplomacyContentContainer`(クライシス一覧等、`update_diplomacy_panel_ui`が
/// 毎回全破棄・再構築する)とは完全に独立したコンテナを使うため、互いのUpdateに
/// 影響しない(既存のP21-013支持UI・スクロールを壊さない)。
#[allow(clippy::too_many_arguments)]
fn update_country_power_info_ui(
    mut commands: Commands,
    state: Res<DiplomacyPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    power_registry: Res<CountryPowerRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    container_q: Query<(Entity, Option<&Children>), With<CountryPowerInfoContainer>>,
) {
    if !state.open && !locale.is_changed() {
        return;
    }
    let Ok((container_entity, children_opt)) = container_q.single() else {
        return;
    };
    let Some(p_cid) = player_country.0 else {
        return;
    };

    if let Some(children) = children_opt {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(container_entity).with_children(|parent| {
        spawn_country_power_block(
            parent,
            &catalog,
            locale.0,
            &country_registry,
            &power_registry,
            p_cid,
            "diplomacy_panel.power_self_heading",
        );

        if let Some(target_cid) = state.target_country
            && target_cid != p_cid
        {
            spawn_country_power_block(
                parent,
                &catalog,
                locale.0,
                &country_registry,
                &power_registry,
                target_cid,
                "diplomacy_panel.power_selected_heading",
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::time::GameDate;
    use crate::common::ClaimId;
    use crate::country::CountryData;
    use crate::diplomacy::claims::TerritorialClaim;
    use crate::state::data::StateData;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<GameNotification>();
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(GameDate::new(1800, 1, 1));
        app.add_systems(
            Update,
            (
                sync_target_country_from_selected_state,
                handle_claim_command_buttons,
                handle_crisis_command_buttons,
                handle_crisis_support_command_buttons,
            )
                .chain(),
        );

        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_resource(SelectedState::default());
        app.insert_resource(DiplomacyPanelState {
            open: true,
            target_country: Some(CountryId(1)),
            claim_target_state: None,
            pending_crisis_claim: None,
            pending_crisis_accept: None,
            pending_crisis_reject: None,
            pending_support_pledge: None,
        });
        app.insert_resource(CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(2),
                    ..CountryData::default()
                },
            ],
        });
        app.insert_resource(StateRegistry::build(vec![
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
            StateData {
                id: StateId(2),
                owner_country_id: CountryId(2),
                ..Default::default()
            },
        ]));
        app.insert_resource(ClaimRegistry::default());
        app.insert_resource(CrisisRegistry::default());
        app.insert_resource(WarJustificationRegistry::default());
        app
    }

    fn press(app: &mut App, bundle: impl Bundle) {
        app.world_mut().spawn((bundle, Interaction::Pressed));
        app.update();
    }

    /// 要求テスト項目7: 外交UI(Command経路)からClaim作成。
    #[test]
    fn claim_command_flow_selects_state_and_creates_claim() {
        let mut app = build_test_app();

        press(
            &mut app,
            ClaimCommandButton(ClaimCommand::SelectState(StateId(1))),
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .claim_target_state,
            Some(StateId(1))
        );

        press(&mut app, ClaimCommandButton(ClaimCommand::Create));

        let claims = &app.world().resource::<ClaimRegistry>().claims;
        assert_eq!(claims.len(), 1);
        let claim = claims.values().next().unwrap();
        assert_eq!(claim.claimant_country, CountryId(0));
        assert_eq!(claim.target_state, StateId(1));
    }

    /// 要求テスト項目8: 対象国変更時に州選択がresetされる。
    #[test]
    fn claim_target_state_resets_when_target_country_changes() {
        let mut app = build_test_app();
        press(
            &mut app,
            ClaimCommandButton(ClaimCommand::SelectState(StateId(1))),
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .claim_target_state,
            Some(StateId(1))
        );

        // StateId(2)はCountryId(2)所有。選択すると対象国がCountryId(2)へ切り替わる。
        app.world_mut().resource_mut::<SelectedState>().0 = Some(StateId(2));
        app.update();

        let diplo = app.world().resource::<DiplomacyPanelState>();
        assert_eq!(diplo.target_country, Some(CountryId(2)));
        assert_eq!(
            diplo.claim_target_state, None,
            "changing target country must clear the stale claim state selection"
        );
    }

    /// 要求テスト項目14: 確認キャンセルでCrisisを作成しない。
    #[test]
    fn crisis_cancel_does_not_create_crisis() {
        let mut app = build_test_app();
        let claim_id =
            app.world_mut()
                .resource_mut::<ClaimRegistry>()
                .add_claim(TerritorialClaim {
                    id: ClaimId(0),
                    claimant_country: CountryId(0),
                    target_state: StateId(1),
                    strength: 50.0,
                    created_date: "1800/01/01".to_string(),
                    is_permanent: false,
                    source: ClaimSource::Strategic,
                    status: crate::diplomacy::claims::ClaimStatus::Active,
                });

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestStart(claim_id)),
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_claim,
            Some(claim_id)
        );

        press(&mut app, CrisisCommandButton(CrisisCommand::Cancel));

        assert!(app.world().resource::<CrisisRegistry>().crises.is_empty());
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_claim,
            None
        );
    }

    /// Crisis確認の2段階目(Confirm)で実際にCrisisが作成される(Cancelとの対比)。
    #[test]
    fn crisis_confirm_after_request_creates_crisis() {
        let mut app = build_test_app();
        let claim_id =
            app.world_mut()
                .resource_mut::<ClaimRegistry>()
                .add_claim(TerritorialClaim {
                    id: ClaimId(0),
                    claimant_country: CountryId(0),
                    target_state: StateId(1),
                    strength: 50.0,
                    created_date: "1800/01/01".to_string(),
                    is_permanent: false,
                    source: ClaimSource::Strategic,
                    status: crate::diplomacy::claims::ClaimStatus::Active,
                });

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestStart(claim_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::Confirm));

        let crises = &app.world().resource::<CrisisRegistry>().crises;
        assert_eq!(crises.len(), 1);
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_claim,
            None,
            "confirming must clear the pending confirmation state"
        );
    }

    /// パネルを閉じると一時的な選択・確認状態が破棄される。
    #[test]
    fn closing_panel_clears_transient_selection() {
        let mut app = build_test_app();
        let mut state = app.world_mut().resource_mut::<DiplomacyPanelState>();
        state.claim_target_state = Some(StateId(1));
        state.pending_crisis_claim = Some(ClaimId(0));
        state.pending_crisis_accept = Some(crate::common::DiplomaticCrisisId(0));
        state.pending_crisis_reject = Some(crate::common::DiplomaticCrisisId(0));
        state.pending_support_pledge = Some((
            crate::common::DiplomaticCrisisId(0),
            crisis_response::CrisisSupportSide::Initiator,
        ));
        state.clear_transient_selection();

        let state = app.world().resource::<DiplomacyPanelState>();
        assert_eq!(state.claim_target_state, None);
        assert_eq!(state.pending_crisis_claim, None);
        assert_eq!(state.pending_crisis_accept, None);
        assert_eq!(state.pending_crisis_reject, None);
        assert_eq!(state.pending_support_pledge, None);
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-011: 最後通牒受諾/拒否/撤回のUIコマンド経路
    // ─────────────────────────────────────────────────────────────────────

    /// プレイヤー(CountryId(0))がtargetとなるDemandSent中のCrisisを直接registryへ
    /// 挿入する(UI経由のRequestStart/Confirmはinitiator視点しか作れないため)。
    fn insert_demand_sent_crisis_targeting_player(
        app: &mut App,
        initiator: CountryId,
        target_state: StateId,
    ) -> crate::common::DiplomaticCrisisId {
        use crate::diplomacy::crisis::{DiplomaticCrisis, WarGoal, WarGoalType};
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(DiplomaticCrisis {
                id: crate::common::DiplomaticCrisisId(0),
                initiator,
                target: CountryId(0),
                war_goals: vec![WarGoal {
                    attacker: initiator,
                    defender: CountryId(0),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![target_state],
                    base_peace_cost: 0.0,
                    international_concern: 0.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1800/01/01".to_string(),
                current_phase: CrisisPhase::DemandSent,
                escalation: 0.0,
                initiator_support: 0.0,
                target_resistance: 0.0,
                days_in_phase: 0,
                deadline_date: Some("1800/01/31".to_string()),
                international_concern: 0.0,
                third_party_reactions: Default::default(),
                related_claim_id: None,
                related_justification_id: None,
                related_war_id: None,
            })
    }

    /// 州CountryId(0)所有のStateId(3)を追加したStateRegistryへ差し替える
    /// (`build_test_app`の初期StateRegistryはCountryId(0)所有州を持たないため)。
    fn add_player_owned_state(app: &mut App) {
        *app.world_mut().resource_mut::<StateRegistry>() = StateRegistry::build(vec![
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                ..Default::default()
            },
            StateData {
                id: StateId(2),
                owner_country_id: CountryId(2),
                ..Default::default()
            },
            StateData {
                id: StateId(3),
                owner_country_id: CountryId(0),
                ..Default::default()
            },
        ]);
    }

    /// 受諾の2段階確認(Confirm)で州が移転しCrisisがResolvedPeacefullyへ遷移する。
    #[test]
    fn crisis_accept_confirm_transfers_state_and_resolves() {
        let mut app = build_test_app();
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestAccept(crisis_id)),
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_accept,
            Some(crisis_id)
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmAccept));

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(StateId(3))
                .unwrap()
                .owner_country_id,
            CountryId(1),
            "accepting must transfer the demanded state to the initiator"
        );
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::ResolvedPeacefully
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_accept,
            None
        );
    }

    /// 受諾確認のキャンセルでは州もCrisisも変化しない。
    #[test]
    fn crisis_accept_cancel_leaves_state_and_crisis_unchanged() {
        let mut app = build_test_app();
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestAccept(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::CancelAccept));

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(StateId(3))
                .unwrap()
                .owner_country_id,
            CountryId(0)
        );
        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_crisis_accept,
            None
        );
    }

    /// 拒否の2段階確認(Confirm)でinitiatorへ完成済みJustificationが付与され、
    /// CrisisがEscalatingへ遷移する。
    #[test]
    fn crisis_reject_confirm_grants_justification_and_escalates() {
        let mut app = build_test_app();
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestReject(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmReject));

        let crisis = app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .clone();
        assert_eq!(crisis.current_phase, CrisisPhase::Escalating);
        let j_id = crisis
            .related_justification_id
            .expect("rejection must grant a justification id");
        assert!(
            app.world()
                .resource::<WarJustificationRegistry>()
                .justifications
                .get(&j_id)
                .unwrap()
                .is_ready
        );
    }

    /// 拒否確認のキャンセルではCrisisは変化しない(Justificationも付与されない)。
    #[test]
    fn crisis_reject_cancel_leaves_crisis_unchanged() {
        let mut app = build_test_app();
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestReject(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::CancelReject));

        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent
        );
        assert!(
            app.world()
                .resource::<WarJustificationRegistry>()
                .justifications
                .is_empty()
        );
    }

    /// initiator視点: DemandSent中の撤回はCancelledへ遷移する(確認なしの単発操作)。
    #[test]
    fn crisis_withdraw_by_initiator_cancels_demand_sent_crisis() {
        let mut app = build_test_app();
        let claim_id =
            app.world_mut()
                .resource_mut::<ClaimRegistry>()
                .add_claim(TerritorialClaim {
                    id: ClaimId(0),
                    claimant_country: CountryId(0),
                    target_state: StateId(1),
                    strength: 50.0,
                    created_date: "1800/01/01".to_string(),
                    is_permanent: false,
                    source: ClaimSource::Strategic,
                    status: crate::diplomacy::claims::ClaimStatus::Active,
                });
        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestStart(claim_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::Confirm));
        let crisis_id = *app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .keys()
            .next()
            .unwrap();

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::Withdraw(crisis_id)),
        );

        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::Cancelled
        );
    }

    /// 第三者(initiatorでもtargetでもないプレイヤー)は撤回できない
    /// (非当事者からのボタン操作に対する安全確認)。
    #[test]
    fn crisis_withdraw_rejects_when_player_is_not_the_initiator() {
        let mut app = build_test_app();
        // initiator=1, target=2 のいずれもプレイヤー(CountryId(0))ではない。
        use crate::diplomacy::crisis::{DiplomaticCrisis, WarGoal, WarGoalType};
        let crisis_id =
            app.world_mut()
                .resource_mut::<CrisisRegistry>()
                .add_crisis(DiplomaticCrisis {
                    id: crate::common::DiplomaticCrisisId(0),
                    initiator: CountryId(1),
                    target: CountryId(2),
                    war_goals: vec![WarGoal {
                        attacker: CountryId(1),
                        defender: CountryId(2),
                        goal_type: WarGoalType::ConquerState,
                        target_states: vec![StateId(2)],
                        base_peace_cost: 0.0,
                        international_concern: 0.0,
                        completion: 0.0,
                        is_primary: true,
                    }],
                    start_date: "1800/01/01".to_string(),
                    current_phase: CrisisPhase::DemandSent,
                    escalation: 0.0,
                    initiator_support: 0.0,
                    target_resistance: 0.0,
                    days_in_phase: 0,
                    deadline_date: Some("1800/01/31".to_string()),
                    international_concern: 0.0,
                    third_party_reactions: Default::default(),
                    related_claim_id: None,
                    related_justification_id: None,
                    related_war_id: None,
                });

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::Withdraw(crisis_id)),
        );

        assert_eq!(
            app.world()
                .resource::<CrisisRegistry>()
                .crises
                .get(&crisis_id)
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent,
            "a non-participant pressing Withdraw must not change the crisis"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-013: 第三国Crisis支持のUIコマンド経路
    // ─────────────────────────────────────────────────────────────────────

    /// initiator=1, target=2 のいずれもプレイヤー(CountryId(0))ではない
    /// (`crisis_withdraw_rejects_when_player_is_not_the_initiator`と同じ設定を
    /// 第三国支持テスト用に再利用する)。
    fn insert_third_party_crisis(app: &mut App) -> DiplomaticCrisisId {
        use crate::diplomacy::crisis::{DiplomaticCrisis, WarGoal, WarGoalType};
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .add_crisis(DiplomaticCrisis {
                id: DiplomaticCrisisId(0),
                initiator: CountryId(1),
                target: CountryId(2),
                war_goals: vec![WarGoal {
                    attacker: CountryId(1),
                    defender: CountryId(2),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(2)],
                    base_peace_cost: 0.0,
                    international_concern: 0.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1800/01/01".to_string(),
                current_phase: CrisisPhase::DemandSent,
                escalation: 0.0,
                initiator_support: 0.0,
                target_resistance: 0.0,
                days_in_phase: 0,
                deadline_date: Some("1800/01/31".to_string()),
                international_concern: 0.0,
                third_party_reactions: Default::default(),
                related_claim_id: None,
                related_justification_id: None,
                related_war_id: None,
            })
    }

    fn support_side_of(
        app: &App,
        crisis_id: DiplomaticCrisisId,
        supporter: CountryId,
    ) -> Option<ThirdCountryReaction> {
        app.world()
            .resource::<CrisisRegistry>()
            .crises
            .get(&crisis_id)
            .unwrap()
            .third_party_reactions
            .get(&supporter)
            .copied()
    }

    /// 要求テスト項目33: 確認後に支持が記録される(要求国側)。
    #[test]
    fn support_confirm_records_initiator_side_pledge() {
        let mut app = build_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestSupportInitiator(crisis_id)),
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_support_pledge,
            Some((crisis_id, crisis_response::CrisisSupportSide::Initiator))
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmSupport));

        assert_eq!(
            support_side_of(&app, crisis_id, CountryId(0)),
            Some(ThirdCountryReaction::SupportsInitiator)
        );
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_support_pledge,
            None,
            "confirming must clear the pending confirmation state"
        );
    }

    /// 要求テスト項目33: 確認後に支持が記録される(対象国側)。
    #[test]
    fn support_confirm_records_target_side_pledge() {
        let mut app = build_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestSupportTarget(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmSupport));

        assert_eq!(
            support_side_of(&app, crisis_id, CountryId(0)),
            Some(ThirdCountryReaction::SupportsTarget)
        );
    }

    /// 要求テスト項目32: 2段階確認のキャンセルで状態不変。
    #[test]
    fn support_cancel_does_not_record_a_pledge() {
        let mut app = build_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestSupportInitiator(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::CancelSupport));

        assert_eq!(support_side_of(&app, crisis_id, CountryId(0)), None);
        assert_eq!(
            app.world()
                .resource::<DiplomacyPanelState>()
                .pending_support_pledge,
            None
        );
    }

    /// 要求テスト項目34: 支持撤回が反映される。
    #[test]
    fn support_withdraw_removes_the_pledge() {
        let mut app = build_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestSupportInitiator(crisis_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmSupport));
        assert!(support_side_of(&app, crisis_id, CountryId(0)).is_some());

        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::WithdrawSupport(crisis_id)),
        );

        assert_eq!(support_side_of(&app, crisis_id, CountryId(0)), None);
    }

    /// initiatorプレイヤー自身は`pledge_support`のドメインAPI自体が拒否するため、
    /// UI経由で押しても記録されない(当事国には支持ボタンを表示しない、の裏付け)。
    #[test]
    fn support_confirm_is_rejected_when_player_is_a_belligerent() {
        let mut app = build_test_app();
        // player(0)がinitiatorのCrisis(既存のクレーム経由フローと同じ設定)。
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));
        // player(0)がtargetの場合で試す(request/confirmはUIコマンドとして直接発行できる)。
        app.world_mut()
            .resource_mut::<DiplomacyPanelState>()
            .pending_support_pledge =
            Some((crisis_id, crisis_response::CrisisSupportSide::Initiator));

        press(&mut app, CrisisCommandButton(CrisisCommand::ConfirmSupport));

        assert_eq!(
            support_side_of(&app, crisis_id, CountryId(0)),
            None,
            "a belligerent must never be recorded as its own crisis's supporter, \
             even if a stale pending_support_pledge somehow points at their own crisis"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-013: 支持ボタンの表示条件・支持国一覧の実描画テスト
    // ─────────────────────────────────────────────────────────────────────

    /// `build_test_app`と同じ資源を使いつつ、`setup_diplomacy_panel`
    /// (Startup)と`update_diplomacy_panel_ui`(Update)を実際に走らせ、
    /// 描画されたUIツリーそのものを検証できるようにする。既存の
    /// コマンド発火テスト(`build_test_app`)は描画を必要としないため、
    /// このテスト群専用に分離する(既存テストの挙動・実行コストを変えないため)。
    fn build_render_test_app() -> App {
        let mut app = build_test_app();
        app.insert_resource(DiplomacyRegistry::default());
        app.insert_resource(WarRegistry::default());
        app.add_systems(
            Startup,
            (crate::ui::tab_bar::spawn_tab_bar, setup_diplomacy_panel).chain(),
        );
        app.add_systems(Update, update_diplomacy_panel_ui);
        app.world_mut().resource_mut::<DiplomacyPanelState>().open = true;
        app.update(); // Startup実行 + 初回描画
        app
    }

    fn has_support_command_button(app: &mut App, command: CrisisCommand) -> bool {
        app.world_mut()
            .query::<&CrisisCommandButton>()
            .iter(app.world())
            .any(|b| b.0 == command)
    }

    /// 要求テスト項目29: 第三国プレイヤーに両支持ボタンを表示する。
    #[test]
    fn third_party_player_sees_both_support_buttons() {
        let mut app = build_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        app.update();

        assert!(has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportInitiator(crisis_id)
        ));
        assert!(has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportTarget(crisis_id)
        ));
    }

    /// 要求テスト項目30: initiatorプレイヤーには支持ボタンを表示しない。
    #[test]
    fn initiator_player_does_not_see_support_buttons() {
        let mut app = build_render_test_app();
        // player(0)がinitiator: 既存のClaim→Crisis確定フローで作る。
        let claim_id =
            app.world_mut()
                .resource_mut::<ClaimRegistry>()
                .add_claim(TerritorialClaim {
                    id: ClaimId(0),
                    claimant_country: CountryId(0),
                    target_state: StateId(1),
                    strength: 50.0,
                    created_date: "1800/01/01".to_string(),
                    is_permanent: false,
                    source: ClaimSource::Strategic,
                    status: crate::diplomacy::claims::ClaimStatus::Active,
                });
        press(
            &mut app,
            CrisisCommandButton(CrisisCommand::RequestStart(claim_id)),
        );
        press(&mut app, CrisisCommandButton(CrisisCommand::Confirm));
        let crisis_id = *app
            .world()
            .resource::<CrisisRegistry>()
            .crises
            .keys()
            .next()
            .unwrap();
        app.update();

        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportInitiator(crisis_id)
        ));
        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportTarget(crisis_id)
        ));
    }

    /// 要求テスト項目31: targetプレイヤーには支持ボタンを表示しない。
    #[test]
    fn target_player_does_not_see_support_buttons() {
        let mut app = build_render_test_app();
        add_player_owned_state(&mut app);
        let crisis_id =
            insert_demand_sent_crisis_targeting_player(&mut app, CountryId(1), StateId(3));
        app.update();

        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportInitiator(crisis_id)
        ));
        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportTarget(crisis_id)
        ));
    }

    /// 要求テスト項目35: terminal Crisisでは支持ボタンを表示しない
    /// (第三国プレイヤーであっても)。
    #[test]
    fn terminal_crisis_shows_no_support_buttons() {
        let mut app = build_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .crises
            .get_mut(&crisis_id)
            .unwrap()
            .current_phase = CrisisPhase::ResolvedPeacefully;
        app.update();

        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportInitiator(crisis_id)
        ));
        assert!(!has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportTarget(crisis_id)
        ));
    }

    /// 要求テスト項目36: 支持国名一覧が表示される(内部CountryIdではなく国名で)。
    #[test]
    fn supporter_country_names_are_rendered() {
        let mut app = build_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        // player(0)ではなく別の第三国(実際にはbuild_test_appに存在しないIDなので、
        // ドメインAPIを直接叩いて確実に登録する)。国名を検証可能にするため、
        // player自身(CountryId(0), CountryData::default()のname="")を支持国にする
        // のではなく、名前付きの国を用意する。
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries
            .push(CountryData {
                id: CountryId(3),
                name: "Testland".to_string(),
                ..CountryData::default()
            });
        let countries = CountryRegistry {
            countries: app.world().resource::<CountryRegistry>().countries.clone(),
        };
        let current_date = app.world().resource::<GameDate>().clone();
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            CountryId(3),
            crisis_response::CrisisSupportSide::Initiator,
            &current_date,
        )
        .unwrap();
        app.update();

        let mut found = false;
        for text in app.world_mut().query::<&Text>().iter(app.world()) {
            if text.0.contains("Testland") {
                found = true;
            }
        }
        assert!(
            found,
            "the supporter's country name must appear somewhere in the rendered crisis list"
        );
    }

    /// 要求テスト項目37: JA/EN切り替えへ即時追従する(支持関連の文言を含む)。
    #[test]
    fn support_ui_text_follows_locale_switch() {
        let mut app = build_render_test_app();
        insert_third_party_crisis(&mut app);
        app.update();

        let ja_texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();

        app.world_mut().resource_mut::<CurrentLocale>().0 = crate::localization::Locale::EnUs;
        app.update();

        let en_texts: Vec<String> = app
            .world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect();

        assert_ne!(
            ja_texts, en_texts,
            "switching locale must change the rendered crisis panel text"
        );
    }

    /// 要求テスト項目39: 支持UI追加後も外交パネルのスクロール本体(scrollable body)が
    /// 正常に構築されていることを回帰確認する(`ScrollPosition`/`RelativeCursorPosition`
    /// を持つ本体が引き続き1つだけ存在する)。
    #[test]
    fn scroll_body_still_present_after_rendering_support_ui() {
        let mut app = build_render_test_app();
        for _ in 0..5 {
            insert_third_party_crisis(&mut app);
        }
        app.update();

        let scroll_body_count = app
            .world_mut()
            .query_filtered::<(&ScrollPosition, &bevy::ui::RelativeCursorPosition), Without<TabBarRoot>>()
            .iter(app.world())
            .count();
        assert_eq!(
            scroll_body_count, 1,
            "the diplomacy panel must still have exactly one scrollable body after P21-013's UI additions"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-014: 国家総合力・国家ランクUIのテスト
    // ─────────────────────────────────────────────────────────────────────

    fn build_power_render_test_app() -> App {
        let mut app = build_render_test_app();
        let power_registry = {
            let world = app.world();
            crate::country::power::evaluate_country_power(
                world.resource::<CountryRegistry>(),
                world.resource::<StateRegistry>(),
                &crate::military::data::MilitaryRegistry::default(),
                &crate::building::data::BuildingRegistry::default(),
                "1800/01/01".to_string(),
            )
        };
        app.insert_resource(power_registry);
        app.add_systems(Update, update_country_power_info_ui);
        app.update();
        app
    }

    fn all_texts(app: &mut App) -> Vec<String> {
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect()
    }

    /// 要求テスト47-49: 選択国のランク・世界順位/国家数・総合力・内訳3要素が表示される。
    #[test]
    fn own_country_power_rank_and_breakdown_are_rendered() {
        let mut app = build_power_render_test_app();
        let texts = all_texts(&mut app);
        let joined = texts.join("\n");

        assert!(
            joined.contains("国家ランク"),
            "power tier line must be rendered: {joined}"
        );
        assert!(
            joined.contains("世界順位"),
            "world rank line must be rendered: {joined}"
        );
        assert!(
            joined.contains("国家総合力"),
            "total score line must be rendered: {joined}"
        );
        assert!(
            joined.contains("軍事力") && joined.contains("経済力") && joined.contains("人口"),
            "all 3 breakdown factors must be rendered: {joined}"
        );
        // build_test_app()のCountryRegistryは3か国(0,1,2)。
        assert!(
            joined.contains("/ 3"),
            "world rank denominator must equal the evaluated country count: {joined}"
        );
    }

    /// 要求テスト50: プレイヤー国家だけでなく、選択した外国の評価も表示される。
    #[test]
    fn selected_foreign_country_power_is_also_rendered() {
        let mut app = build_power_render_test_app();
        // build_test_app()の既定でplayer=CountryId(0)、target_country=CountryId(1)
        // (=自国と異なる第三国)であり、両方のheadingが描画されるはず。
        let texts = all_texts(&mut app);
        let self_heading_count = texts.iter().filter(|t| t.starts_with("自国:")).count();
        let selected_heading_count = texts.iter().filter(|t| t.starts_with("選択国:")).count();

        assert_eq!(
            self_heading_count, 1,
            "own-country heading must render exactly once"
        );
        assert_eq!(
            selected_heading_count, 1,
            "selected foreign country heading must render exactly once when target != player"
        );
    }

    /// 要求テスト51: 評価未構築(空のCountryPowerRegistry)でもpanicせず、
    /// 安全な「評価中」表示にフォールバックする。
    #[test]
    fn unevaluated_country_falls_back_to_pending_display_without_panicking() {
        let mut app = build_render_test_app();
        app.insert_resource(CountryPowerRegistry::default());
        app.add_systems(Update, update_country_power_info_ui);
        app.update(); // panicしないこと自体がテスト

        let texts = all_texts(&mut app);
        let joined = texts.join("\n");
        assert!(
            joined.contains("評価中"),
            "an unevaluated country must show the pending fallback text: {joined}"
        );
    }

    /// 要求テスト52: JA/EN切り替えで即時反映される。
    #[test]
    fn power_ui_text_follows_locale_switch() {
        let mut app = build_power_render_test_app();
        let ja_texts = all_texts(&mut app);

        app.world_mut().resource_mut::<CurrentLocale>().0 = crate::localization::Locale::EnUs;
        app.update();
        let en_texts = all_texts(&mut app);

        assert_ne!(
            ja_texts, en_texts,
            "switching locale must change the rendered power info text"
        );
        assert!(en_texts.iter().any(|t| t.contains("Power tier")));
    }

    /// 要求テスト53-54: 小数1桁表示、NaN/infは表示しない。
    #[test]
    fn format_power_value_uses_one_decimal_and_never_shows_nan_or_inf() {
        assert_eq!(format_power_value(73.44), "73.4");
        assert_eq!(format_power_value(0.0), "0.0");
        assert_eq!(format_power_value(100.0), "100.0");
        assert_eq!(format_power_value(f32::NAN), "-");
        assert_eq!(format_power_value(f32::INFINITY), "-");
        assert_eq!(format_power_value(f32::NEG_INFINITY), "-");
        assert_eq!(
            format_power_value(-5.0),
            "0.0",
            "negative values must clamp to 0"
        );
    }

    /// 要求テスト55: 国家総合力UI追加後もスクロール本体が1つだけ存在する。
    #[test]
    fn scroll_body_still_present_after_rendering_power_ui() {
        let mut app = build_power_render_test_app();
        let scroll_body_count = app
            .world_mut()
            .query_filtered::<(&ScrollPosition, &bevy::ui::RelativeCursorPosition), Without<TabBarRoot>>()
            .iter(app.world())
            .count();
        assert_eq!(scroll_body_count, 1);
    }

    /// 要求テスト56: 国家総合力UI追加後もP21-013の支持ボタンが維持されている。
    #[test]
    fn support_buttons_still_render_alongside_power_ui() {
        let mut app = build_power_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        app.update();

        assert!(has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportInitiator(crisis_id)
        ));
        assert!(has_support_command_button(
            &mut app,
            CrisisCommand::RequestSupportTarget(crisis_id)
        ));
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-015: AI大国の支持が既存P21-013支持者一覧UIへそのまま表示されることの確認
    // ─────────────────────────────────────────────────────────────────────

    /// 要求テスト45: AI(`crisis_response::pledge_support`を通じて追加された支持、
    /// プレイヤー操作を経由しない点だけがP21-013のプレイヤー手動支持と異なる)による
    /// 支持も、既存の支持者一覧UIへコード変更無しでそのまま表示される
    /// (`third_party_reactions`はプレイヤー支持もAI支持も同じ`ThirdCountryReaction`
    /// として区別なく保持されるため、レンダラー側は元々どちらかを判定できない設計)。
    #[test]
    fn ai_pledged_support_is_rendered_by_the_existing_p21_013_supporter_list_ui() {
        let mut app = build_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);

        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries
            .push(CountryData {
                id: CountryId(3),
                name: "AI Great Power".to_string(),
                ..CountryData::default()
            });
        let countries = CountryRegistry {
            countries: app.world().resource::<CountryRegistry>().countries.clone(),
        };
        let current_date = app.world().resource::<GameDate>().clone();
        // UIボタンを一切経由せず、AI側のオーケストレーション関数
        // (`diplomacy::update::evaluate_ai_crisis_support`)と全く同じドメインAPI呼び出しで
        // 支持を追加する。
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            CountryId(3),
            crisis_response::CrisisSupportSide::Initiator,
            &current_date,
        )
        .unwrap();
        app.update();

        let mut found = false;
        for text in app.world_mut().query::<&Text>().iter(app.world()) {
            if text.0.contains("AI Great Power") {
                found = true;
            }
        }
        assert!(
            found,
            "an AI-pledged supporter's name must appear in the rendered crisis list, \
             exactly like a player-pledged one"
        );
    }

    /// 要求テスト46: AI支持がいても、外交パネルのスクロールとP21-014の国家ランク表示が
    /// 両方とも維持される。
    #[test]
    fn scroll_and_power_ui_survive_alongside_ai_pledged_support() {
        let mut app = build_power_render_test_app();
        let crisis_id = insert_third_party_crisis(&mut app);
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .countries
            .push(CountryData {
                id: CountryId(3),
                name: "AI Great Power".to_string(),
                ..CountryData::default()
            });
        let countries = CountryRegistry {
            countries: app.world().resource::<CountryRegistry>().countries.clone(),
        };
        let current_date = app.world().resource::<GameDate>().clone();
        crisis_response::pledge_support(
            &mut app.world_mut().resource_mut::<CrisisRegistry>(),
            &countries,
            crisis_id,
            CountryId(3),
            crisis_response::CrisisSupportSide::Target,
            &current_date,
        )
        .unwrap();
        app.update();

        let scroll_body_count = app
            .world_mut()
            .query_filtered::<(&ScrollPosition, &bevy::ui::RelativeCursorPosition), Without<TabBarRoot>>()
            .iter(app.world())
            .count();
        assert_eq!(scroll_body_count, 1);

        let texts = all_texts(&mut app);
        let joined = texts.join("\n");
        assert!(
            joined.contains("国家ランク") && joined.contains("世界順位"),
            "P21-014's power info block must still render: {joined}"
        );
    }

    fn build_toggle_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::ui::ActivePanel::default());
        app.insert_resource(DiplomacyPanelState::default());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_systems(Update, toggle_diplomacy_panel_key);
        app
    }

    fn press_ctrl_plus(app: &mut App, digit: KeyCode) {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ControlLeft);
        keys.press(digit);
        app.insert_resource(keys);
        app.update();
    }

    fn press_bare(app: &mut App, digit: KeyCode) {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(digit);
        app.insert_resource(keys);
        app.update();
    }

    #[test]
    fn ctrl_plus_digit3_toggles_diplomacy_panel() {
        let mut app = build_toggle_test_app();
        press_ctrl_plus(&mut app, KeyCode::Digit3);

        assert!(app.world().resource::<DiplomacyPanelState>().open);
        assert_eq!(
            app.world().resource::<crate::ui::ActivePanel>().current,
            crate::ui::PanelKind::Diplomacy
        );
    }

    #[test]
    fn bare_digit3_alone_does_not_toggle_diplomacy_panel() {
        let mut app = build_toggle_test_app();
        press_bare(&mut app, KeyCode::Digit3);

        assert!(
            !app.world().resource::<DiplomacyPanelState>().open,
            "Digit3 without Ctrl must not open the Diplomacy panel"
        );
    }

    #[test]
    fn toggle_button_is_spawned_as_a_child_of_the_shared_tab_bar() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(
            Startup,
            (crate::ui::tab_bar::spawn_tab_bar, setup_diplomacy_panel).chain(),
        );
        app.update();

        let tab_bar = app
            .world_mut()
            .query_filtered::<Entity, With<crate::ui::tab_bar::TabBarRoot>>()
            .single(app.world())
            .expect("TabBarRoot must be spawned");
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<ToggleDiplomacyPanelButton>>()
            .single(app.world())
            .expect("ToggleDiplomacyPanelButton must be spawned");

        assert_eq!(
            app.world().entity(button).get::<ChildOf>().map(|c| c.0),
            Some(tab_bar),
            "the diplomacy toggle button must be a child of the shared TabBarRoot"
        );
    }

    /// P21-013: `DiplomacyPanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button以外の余白をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。
    #[test]
    fn diplomacy_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_diplomacy_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<DiplomacyPanelRoot>>()
            .single(app.world())
            .expect("DiplomacyPanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "DiplomacyPanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }
}
