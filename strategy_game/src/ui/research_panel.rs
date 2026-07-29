use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::research::allocation::InProgressTech;
use crate::research::data::{TechnologyField, TechnologyRegistry};
use crate::research::world_stage::WorldCivilizationState;
use crate::ui::{ActivePanel, PanelKind};
use bevy::prelude::*;

#[derive(Component)]
pub struct ResearchPanelRoot;

#[derive(Resource, Default)]
pub struct ResearchPanelState {
    pub open: bool,
    pub active_tab: TechnologyField,
}

#[derive(Component)]
pub struct ResearchTabButton(pub TechnologyField);

#[derive(Component)]
pub struct AllocationAdjustButton(pub TechnologyField, pub f32); // +0.05 or -0.05

#[derive(Component)]
pub struct StartTechButton(pub String, pub TechnologyField);

#[derive(Component)]
pub struct CancelTechButton(pub TechnologyField);

#[derive(Component)]
pub struct ToggleResearchPanelButton;

#[derive(Component)]
pub struct ResearchHeaderText;

#[derive(Component)]
pub struct TechListContainer;

pub struct ResearchPluginUI;

impl Plugin for ResearchPluginUI {
    fn build(&self, app: &mut App) {
        app.insert_resource(ResearchPanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_research_panel)
            .add_systems(
                Update,
                (
                    toggle_research_panel_key,
                    handle_tab_buttons,
                    handle_allocation_buttons,
                    handle_tech_action_buttons,
                    update_research_panel_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_research_panel(mut commands: Commands) {
    // 画面左上に表示ボタンを配置
    commands
        .spawn((
            ToggleResearchPanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.2, 0.3, 0.5, 0.9)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("[R] Research Panel"),
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
            ResearchPanelRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(75.0),
                width: Val::Px(580.0),
                height: Val::Px(600.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                display: Display::None,
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.08, 0.12, 0.95)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("-- Technology & Research --"),
                TextColor(Color::srgb(0.9, 0.85, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            parent.spawn((
                ResearchHeaderText,
                Text::new(""),
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Alloc (+5%):"),
                        TextColor(Color::srgb(0.8, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));

                    for field in TechnologyField::ALL {
                        row.spawn((
                            AllocationAdjustButton(field, 0.05),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.3, 0.35, 0.45, 1.0)),
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new(format!("+{}", field.display_name())),
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
                    column_gap: Val::Px(4.0),
                    margin: UiRect::top(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|tab_row| {
                    for field in TechnologyField::ALL {
                        tab_row
                            .spawn((
                                ResearchTabButton(field),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.25, 0.35, 1.0)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new(field.display_name()),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(13.0),
                                        ..default()
                                    },
                                ));
                            });
                    }
                });

            parent.spawn((
                TechListContainer,
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

fn toggle_research_panel_key(
    mut state: ResMut<ResearchPanelState>,
    mut active_panel: ResMut<ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<ToggleResearchPanelButton>, Changed<Interaction>)>,
    mut panel_q: Query<&mut Node, With<ResearchPanelRoot>>,
) {
    let mut toggle = false;
    if keys.just_pressed(KeyCode::KeyR) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        active_panel.toggle(PanelKind::Research);
        state.open = active_panel.current == PanelKind::Research;
        if let Ok(mut node) = panel_q.single_mut() {
            node.display = if state.open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

fn handle_tab_buttons(
    mut interaction_q: Query<(&Interaction, &ResearchTabButton), Changed<Interaction>>,
    mut state: ResMut<ResearchPanelState>,
) {
    for (interaction, btn) in interaction_q.iter_mut() {
        if *interaction == Interaction::Pressed {
            state.active_tab = btn.0;
        }
    }
}

fn handle_allocation_buttons(
    mut interaction_q: Query<(&Interaction, &AllocationAdjustButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
) {
    let Some(cid) = player_country.0 else { return };
    let Some(country) = country_registry.get_mut(cid) else {
        return;
    };

    for (interaction, btn) in interaction_q.iter_mut() {
        if *interaction == Interaction::Pressed {
            let current = country.research_state.allocation.get(btn.0);
            country
                .research_state
                .allocation
                .set(btn.0, current + btn.1);
        }
    }
}

fn handle_tech_action_buttons(
    start_q: Query<(&Interaction, &StartTechButton), Changed<Interaction>>,
    cancel_q: Query<(&Interaction, &CancelTechButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    world_state: Res<WorldCivilizationState>,
) {
    let Some(cid) = player_country.0 else { return };
    let Some(country) = country_registry.get_mut(cid) else {
        return;
    };

    for (interaction, btn) in start_q.iter() {
        if *interaction == Interaction::Pressed {
            if country.research_state.in_progress.contains_key(&btn.1) {
                continue;
            }
            if let Some(def) = tech_registry.get(&btn.0)
                && def.minimum_world_stage <= world_state.current_stage
                && def
                    .prerequisites
                    .iter()
                    .all(|pre| country.research_state.completed_technologies.contains(pre))
            {
                country.research_state.in_progress.insert(
                    btn.1,
                    InProgressTech {
                        tech_id: btn.0.clone(),
                        progress: 0.0,
                        cost: def.cost,
                    },
                );
            }
        }
    }

    for (interaction, btn) in cancel_q.iter() {
        if *interaction == Interaction::Pressed {
            country.research_state.in_progress.remove(&btn.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_research_panel_ui(
    mut commands: Commands,
    state: Res<ResearchPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    world_state: Res<WorldCivilizationState>,
    mut header_q: Query<&mut Text, With<ResearchHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<TechListContainer>>,
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

    let Some(cid) = player_country.0 else { return };
    let Some(country) = country_registry.get(cid) else {
        return;
    };

    let active_field = state.active_tab;
    let alloc = &country.research_state.allocation;

    let header_info = format!(
        "World Era: {} | Sci Cap: {:.1} | Mag Cap: {:.1}\nAllocations: Sci {:.0}% | Mag {:.0}% | Mil {:.0}% | Fus {:.0}%",
        world_state.current_stage.display_name(),
        country.science_research_capacity,
        country.magic_research_capacity,
        alloc.science * 100.0,
        alloc.magic * 100.0,
        alloc.military * 100.0,
        alloc.fusion * 100.0,
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
        if let Some(in_prog) = country.research_state.in_progress.get(&active_field) {
            let tech_name = tech_registry
                .get(&in_prog.tech_id)
                .map(|d| d.name.as_str())
                .unwrap_or("Unknown");

            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(6.0)),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .insert(BackgroundColor(Color::srgba(0.2, 0.3, 0.2, 0.9)))
                .with_children(|row| {
                    row.spawn((
                        Text::new(format!(
                            "Researching: {} ({:.0}/{:.0} pts)",
                            tech_name, in_prog.progress, in_prog.cost
                        )),
                        TextColor(Color::srgb(0.8, 1.0, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));

                    row.spawn((
                        CancelTechButton(active_field),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.6, 0.2, 0.2, 1.0)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("Cancel"),
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
                });
        }

        for tech_id in &tech_registry.sorted_ids {
            if let Some(def) = tech_registry.get(tech_id) {
                if def.field != active_field {
                    continue;
                }

                let is_done = country
                    .research_state
                    .completed_technologies
                    .contains(tech_id);
                let is_in_prog = country
                    .research_state
                    .in_progress
                    .values()
                    .any(|p| p.tech_id == *tech_id);
                let era_req_met = def.minimum_world_stage <= world_state.current_stage;
                let prereq_met = def
                    .prerequisites
                    .iter()
                    .all(|p| country.research_state.completed_technologies.contains(p));

                let can_start = !is_done
                    && !is_in_prog
                    && era_req_met
                    && prereq_met
                    && !country
                        .research_state
                        .in_progress
                        .contains_key(&active_field);

                let status_label = if is_done {
                    "[Completed]"
                } else if is_in_prog {
                    "[In Progress]"
                } else if !era_req_met {
                    "[Locked: Era Req]"
                } else if !prereq_met {
                    "[Locked: Prereq Req]"
                } else {
                    "[Available]"
                };

                parent
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        padding: UiRect::all(Val::Px(6.0)),
                        margin: UiRect::bottom(Val::Px(2.0)),
                        ..default()
                    })
                    .insert(BackgroundColor(Color::srgba(0.15, 0.15, 0.2, 0.9)))
                    .with_children(|row| {
                        row.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            ..default()
                        })
                        .with_children(|col| {
                            col.spawn((
                                Text::new(format!(
                                    "{} {} (Cost: {:.0})",
                                    def.name, status_label, def.cost
                                )),
                                TextColor(if is_done {
                                    Color::srgb(0.6, 0.8, 0.6)
                                } else {
                                    Color::WHITE
                                }),
                                TextFont {
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                            ));
                            col.spawn((
                                Text::new(&def.description),
                                TextColor(Color::srgb(0.7, 0.7, 0.7)),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));
                        });

                        if can_start {
                            row.spawn((
                                StartTechButton(def.id.clone(), active_field),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.2, 0.5, 0.2, 1.0)),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new("Start"),
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
        }
    });
}
