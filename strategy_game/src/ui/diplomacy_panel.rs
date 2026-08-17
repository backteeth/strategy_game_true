use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::{ClaimId, CountryId, StateId};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::{ClaimRegistry, ClaimSource};
use crate::diplomacy::crisis::{CrisisPhase, CrisisRegistry};
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
}

impl DiplomacyPanelState {
    /// P21-010: 一時的な請求州選択・危機開始確認状態だけを破棄する
    /// (`open`/`target_country`は対象外)。
    fn clear_transient_selection(&mut self) {
        self.claim_target_state = None;
        self.pending_crisis_claim = None;
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
}

#[derive(Component)]
pub struct CrisisCommandButton(pub CrisisCommand);

#[derive(Component)]
pub struct DiplomacyHeaderText;

#[derive(Component)]
pub struct DiplomacyContentContainer;

pub struct DiplomacyPluginUI;

impl Plugin for DiplomacyPluginUI {
    fn build(&self, app: &mut App) {
        app.insert_resource(DiplomacyPanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_diplomacy_panel)
            .add_systems(
                Update,
                (
                    toggle_diplomacy_panel_key,
                    sync_target_country_from_selected_state,
                    handle_diplomacy_action_buttons,
                    handle_claim_command_buttons,
                    handle_crisis_command_buttons,
                    update_diplomacy_panel_ui,
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
) {
    commands
        .spawn((
            ToggleDiplomacyPanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(570.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.2, 0.4, 0.3, 0.9)),
        ))
        .with_children(|parent| {
            let (text, marker) =
                localized_text(&catalog, locale.0, "diplomacy_panel.toggle_button", vec![]);
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

    commands
        .spawn((
            DiplomacyPanelRoot,
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
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.1, 0.08, 0.95)),
        ))
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

            parent.spawn((
                DiplomacyContentContainer,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    overflow: Overflow::clip_y(),
                    flex_grow: 1.0,
                    ..default()
                },
            ));
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
    // KeyGを使用(KeyDはWASDカメラ移動の右パンと衝突するため割り当てない)。
    if keys.just_pressed(KeyCode::KeyG) {
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
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
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
                        &catalog,
                        locale.0,
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
                        &catalog,
                        locale.0,
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
                            &catalog,
                            locale.0,
                            "diplomacy_panel.notif_proposal_accepted",
                            vec![
                                ("treaty", t(&catalog, locale.0, treaty_type.display_name())),
                                ("country", target.name.clone()),
                            ],
                        ),
                    });
                } else {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.notif_proposal_rejected",
                            vec![
                                ("treaty", t(&catalog, locale.0, treaty_type.display_name())),
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
                        &catalog,
                        locale.0,
                        "diplomacy_panel.notif_treaty_broken",
                        vec![
                            ("treaty", t(&catalog, locale.0, treaty_type.display_name())),
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
                        .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.notif_justification_started",
                            vec![("state", st_name)],
                        ),
                    });
                }
                Err(err) => {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "diplomacy_panel.notif_justify_failed",
                            vec![("reason", t(&catalog, locale.0, err))],
                        ),
                    });
                }
            }
        }
    }

    for (interaction, btn) in dec_q.iter() {
        if *interaction == Interaction::Pressed {
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
                    let target_name = country_registry
                        .get(btn.0)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
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
                            &catalog,
                            locale.0,
                            "diplomacy_panel.notif_declare_failed",
                            vec![("reason", t(&catalog, locale.0, err))],
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
    claim_registry: Res<ClaimRegistry>,
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

                let mut my_crises: Vec<_> = crisis_registry
                    .crises
                    .values()
                    .filter(|c| c.initiator == p_cid || c.target == p_cid)
                    .collect();
                my_crises.sort_by_key(|c| c.id.0);

                if my_crises.is_empty() {
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
                    for crisis in my_crises {
                        let other_id = if crisis.initiator == p_cid {
                            crisis.target
                        } else {
                            crisis.initiator
                        };
                        let other_name = country_registry
                            .get(other_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
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
                        sec.spawn((
                            text,
                            marker,
                            TextColor(Color::srgb(0.9, 0.7, 0.9)),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
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
                    for war in active_wars {
                        let attacker_names: Vec<_> = war
                            .attackers
                            .iter()
                            .filter_map(|cid| country_registry.get(*cid).map(|c| c.name.as_str()))
                            .collect();
                        let defender_names: Vec<_> = war
                            .defenders
                            .iter()
                            .filter_map(|cid| country_registry.get(*cid).map(|c| c.name.as_str()))
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
        state.clear_transient_selection();

        let state = app.world().resource::<DiplomacyPanelState>();
        assert_eq!(state.claim_target_state, None);
        assert_eq!(state.pending_crisis_claim, None);
    }
}
