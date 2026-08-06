use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::CountryId;
use crate::country::{CountryRegistry, PlayerCountry};
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
                    update_diplomacy_panel_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
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
        state.open = !state.open;
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
    {
        diplo_state.target_country = Some(state.owner_country_id);
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
