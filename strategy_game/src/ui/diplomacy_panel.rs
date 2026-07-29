use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::CountryId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::data::{
    ActiveDiplomaticActivity, ActiveTreaty, DiplomacyRegistry, DiplomaticActivityType, TreatyType,
};
use crate::diplomacy::proposal::calculate_proposal_score;
use crate::diplomacy::update::ACTIVITY_DURATION_DAYS;
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

fn setup_diplomacy_panel(mut commands: Commands) {
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
            parent.spawn((
                Text::new("[D] Diplomacy Panel"),
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
            parent.spawn((
                Text::new("-- Foreign Affairs & Diplomacy --"),
                TextColor(Color::srgb(0.6, 0.9, 0.7)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            parent.spawn((
                DiplomacyHeaderText,
                Text::new(""),
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
    if keys.just_pressed(KeyCode::KeyD) {
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
                    message: format!(
                        "Diplomacy Started: Improve Relations with Country #{}",
                        target_cid.0
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
                    message: format!(
                        "Diplomacy Started: Harm Relations with Country #{}",
                        target_cid.0
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
                        message: format!(
                            "Proposal Accepted: {} signed with {}!",
                            treaty_type.display_name(),
                            target.name
                        ),
                    });
                } else {
                    notif_writer.write(GameNotification {
                        message: format!(
                            "Proposal Rejected: {} was declined by {}.",
                            treaty_type.display_name(),
                            target.name
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
                    message: format!(
                        "Treaty Broken: {} with Country #{}. Opinion -25",
                        treaty_type.display_name(),
                        target_cid.0
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
                        .map(|s| s.name.as_str())
                        .unwrap_or("State");
                    notif_writer.write(GameNotification {
                        message: format!("Started War Justification for {}!", st_name),
                    });
                }
                Err(err) => {
                    notif_writer.write(GameNotification {
                        message: format!("Cannot Justify War: {}", err),
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
                        .map(|c| c.name.as_str())
                        .unwrap_or("Country");
                    notif_writer.write(GameNotification {
                        message: format!("DECLARED WAR on {}! (War ID: {:?})", target_name, war_id),
                    });
                }
                Err(err) => {
                    notif_writer.write(GameNotification {
                        message: format!("Cannot Declare War: {}", err),
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
    mut header_q: Query<&mut Text, With<DiplomacyHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<DiplomacyContentContainer>>,
) {
    if !state.open {
        return;
    }

    let Ok(mut header_text) = header_q.single_mut() else {
        return;
    };
    let Ok((container_entity, children_opt)) = container_q.single() else {
        return;
    };

    let Some(p_cid) = player_country.0 else {
        return;
    };
    let Some(target_cid) = state.target_country else {
        *header_text = Text::new("Select a foreign state/country on the map.");
        if let Some(children) = children_opt {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        return;
    };

    if target_cid == p_cid {
        *header_text =
            Text::new("Selected self: Cannot perform foreign diplomacy on your own country.");
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

    let header_info = format!(
        "Diplomacy with: {} (ID: {})\nOpinion: {:+.1} / 100.0",
        target.name, target.id.0, rel.opinion
    );
    if header_text.0 != header_info {
        *header_text = Text::new(header_info);
    }

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
                    let side = if act.initiator == p_cid {
                        "Initiated by you"
                    } else {
                        "Initiated by foreign country"
                    };
                    col.spawn((
                        Text::new(format!(
                            "Active Activity: {} ({}d remaining) [{}]",
                            act.activity_type.display_name(),
                            act.days_remaining,
                            side
                        )),
                        TextColor(Color::srgb(0.9, 0.9, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                } else {
                    col.spawn((
                        Text::new("Active Activity: None"),
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));
                }

                if let Some(&cd) = rel.cooldowns.get(&p_cid) {
                    col.spawn((
                        Text::new(format!("Cooldown Active: {} days remaining", cd)),
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
                col.spawn((
                    Text::new("[ Signed Treaties ]"),
                    TextColor(Color::srgb(0.6, 0.9, 0.7)),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                ));

                let active_treaties: Vec<_> = rel.treaties.iter().filter(|t| t.is_active).collect();
                if active_treaties.is_empty() {
                    col.spawn((
                        Text::new("  None"),
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else {
                    for t in active_treaties {
                        col.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                Text::new(format!(
                                    "  • {} (Signed: {})",
                                    t.treaty_type.display_name(),
                                    t.signed_date
                                )),
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            row.spawn((
                                BreakTreatyButton(target_cid, t.treaty_type),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.6, 0.2, 0.2, 1.0)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new("Break"),
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
                        b.spawn((
                            Text::new("Improve Relations (+30d)"),
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
                        b.spawn((
                            Text::new("Harm Relations (-30d)"),
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
                } else {
                    let reason = if act_active {
                        "Activity in progress"
                    } else {
                        "Cooldown active"
                    };
                    row.spawn((
                        Text::new(format!("[Actions Disabled: {}]", reason)),
                        TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                }
            });

        parent.spawn((
            Text::new("[ Propose Treaties & Score Breakdown ]"),
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
                        row.spawn((
                            Text::new(format!(
                                "Propose {}: Total Score {:.1} / Req {:.1} -> {}",
                                proposal.display_name(),
                                breakdown.total_score,
                                breakdown.required_score,
                                if already_signed {
                                    "Already Signed"
                                } else if breakdown.accepted {
                                    "ACCEPT"
                                } else {
                                    "REJECT"
                                }
                            )),
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
                                b.spawn((
                                    Text::new("Propose"),
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

                    col.spawn((
                        Text::new(format!("  Breakdown: {}", item_str)),
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
                sec.spawn((
                    Text::new("-- War & Justification --"),
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
                    sec.spawn((
                        Text::new("Currently at War with this country!"),
                        TextColor(Color::srgb(1.0, 0.3, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else if has_alliance {
                    sec.spawn((
                        Text::new("Cannot declare war / justify: Alliance is active."),
                        TextColor(Color::srgb(0.9, 0.6, 0.3)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                } else if has_nap {
                    sec.spawn((
                        Text::new("Cannot declare war / justify: Non-Aggression Pact is active."),
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
                        sec.spawn((
                            Text::new("Target country owns no states."),
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
                                        row.spawn((
                                            Text::new(format!("Goal: Take {} [READY]", st.name)),
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
                                                b.spawn((
                                                    Text::new("DECLARE WAR"),
                                                    TextColor(Color::WHITE),
                                                    TextFont {
                                                        font_size: FontSize::Px(10.0),
                                                        ..default()
                                                    },
                                                ));
                                            },
                                        );
                                    } else if let Some(j) = active_just {
                                        row.spawn((
                                            Text::new(format!(
                                                "Justifying for {}: {}/{} days",
                                                st.name, j.days_passed, j.required_days
                                            )),
                                            TextColor(Color::srgb(0.9, 0.8, 0.3)),
                                            TextFont {
                                                font_size: FontSize::Px(11.0),
                                                ..default()
                                            },
                                        ));
                                    } else {
                                        row.spawn((
                                            Text::new(format!("Target State: {}", st.name)),
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
                                                b.spawn((
                                                    Text::new("Justify War"),
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
                sec.spawn((
                    Text::new("-- Active Wars --"),
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
                    sec.spawn((
                        Text::new("No active wars in the world."),
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

                        sec.spawn((
                            Text::new(format!(
                                "[War #{:?}] {} ({} vs {}) - Started: {}",
                                war.id.0,
                                war.name,
                                attacker_names.join(", "),
                                defender_names.join(", "),
                                war.start_date
                            )),
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
