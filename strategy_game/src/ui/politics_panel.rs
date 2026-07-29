use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::politics::interest_groups::InterestGroupType;
use crate::politics::reform::PoliticalReform;
use crate::politics::values::ValueAxis;
use crate::ui::{ActivePanel, PanelKind};
use bevy::prelude::*;

#[derive(Component)]
pub struct PoliticsPanelRoot;

#[derive(Resource, Default)]
pub struct PoliticsPanelState {
    pub open: bool,
}

#[derive(Component)]
pub struct TogglePoliticsPanelButton;

#[derive(Component)]
pub struct StartReformButton(pub ValueAxis, pub f32); // axis, delta (+10.0 or -10.0)

#[derive(Component)]
pub struct CancelReformButton;

#[derive(Component)]
pub struct PoliticsHeaderText;

#[derive(Component)]
pub struct PoliticsListContainer;

pub struct PoliticsPluginUI;

impl Plugin for PoliticsPluginUI {
    fn build(&self, app: &mut App) {
        app.insert_resource(PoliticsPanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_politics_panel)
            .add_systems(
                Update,
                (
                    toggle_politics_panel_key,
                    handle_reform_buttons,
                    update_politics_panel_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_politics_panel(mut commands: Commands) {
    // 画面左上に表示ボタンを配置
    commands
        .spawn((
            TogglePoliticsPanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(440.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.5, 0.2, 0.3, 0.9)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("[P] Politics Panel"),
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
            PoliticsPanelRoot,
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
            BackgroundColor(Color::srgba(0.12, 0.08, 0.1, 0.95)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("-- Government & Politics --"),
                TextColor(Color::srgb(0.9, 0.7, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            parent.spawn((
                PoliticsHeaderText,
                Text::new(""),
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            parent.spawn((
                PoliticsListContainer,
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

fn toggle_politics_panel_key(
    mut state: ResMut<PoliticsPanelState>,
    mut active_panel: ResMut<ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<TogglePoliticsPanelButton>, Changed<Interaction>)>,
    mut panel_q: Query<&mut Node, With<PoliticsPanelRoot>>,
) {
    let mut toggle = false;
    if keys.just_pressed(KeyCode::KeyP) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        active_panel.toggle(PanelKind::Politics);
        state.open = active_panel.current == PanelKind::Politics;
        if let Ok(mut node) = panel_q.single_mut() {
            node.display = if state.open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

fn handle_reform_buttons(
    start_q: Query<(&Interaction, &StartReformButton), Changed<Interaction>>,
    cancel_q: Query<(&Interaction, &CancelReformButton), Changed<Interaction>>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
) {
    let Some(cid) = player_country.0 else { return };
    let Some(country) = country_registry.get_mut(cid) else {
        return;
    };

    for (interaction, btn) in start_q.iter() {
        if *interaction == Interaction::Pressed && country.current_reform.is_none() {
            let current_val = match btn.0 {
                ValueAxis::ScienceMagic => country.politics.values.science_magic,
                ValueAxis::IndividualState => country.politics.values.individual_state,
                ValueAxis::SecularReligious => country.politics.values.secular_religious,
            };
            country.current_reform = Some(PoliticalReform::new(btn.0, current_val, btn.1));
        }
    }

    for (interaction, _) in cancel_q.iter() {
        if *interaction == Interaction::Pressed {
            country.current_reform = None;
        }
    }
}

fn update_politics_panel_ui(
    mut commands: Commands,
    state: Res<PoliticsPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    mut header_q: Query<&mut Text, With<PoliticsHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<PoliticsListContainer>>,
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

    let pol = &country.politics;
    let vals = &pol.values;

    let header_info = format!(
        "Gov: {} (Locked) | Econ: {} (Locked)\nValues: Sci/Mag {:.0} | Ind/State {:.0} | Sec/Rel {:.0}",
        country.government_type.display_name(),
        country.economic_system.display_name(),
        vals.science_magic,
        vals.individual_state,
        vals.secular_religious
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
        if let Some(ref reform) = country.current_reform {
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(Val::Px(6.0)),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .insert(BackgroundColor(Color::srgba(0.3, 0.2, 0.2, 0.9)))
                .with_children(|row| {
                    row.spawn((
                        Text::new(format!(
                            "Reform: {} -> {:.0} ({:.0}/{:.0} - {:.1}/mo, Res: {:.1})",
                            reform.axis.display_name(),
                            reform.target_value,
                            reform.progress,
                            reform.required_progress,
                            reform.monthly_progress,
                            reform.clergy_resistance
                        )),
                        TextColor(Color::srgb(1.0, 0.8, 0.8)),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));

                    row.spawn((
                        CancelReformButton,
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
        } else {
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|col| {
                    col.spawn((
                        Text::new("[ Start Value Reform ]"),
                        TextColor(Color::srgb(0.9, 0.85, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));

                    let axes = [
                        (
                            ValueAxis::ScienceMagic,
                            "Science (-10)",
                            -10.0,
                            "Magic (+10)",
                            10.0,
                        ),
                        (
                            ValueAxis::IndividualState,
                            "Individual (-10)",
                            -10.0,
                            "State (+10)",
                            10.0,
                        ),
                        (
                            ValueAxis::SecularReligious,
                            "Secular (-10)",
                            -10.0,
                            "Religion (+10)",
                            10.0,
                        ),
                    ];

                    for (axis, label_minus, delta_minus, label_plus, delta_plus) in axes {
                        col.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                Text::new(format!("{:15}:", axis.display_name())),
                                TextColor(Color::WHITE),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                            ));

                            row.spawn((
                                StartReformButton(axis, delta_minus),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.3, 0.4, 1.0)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(label_minus),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            });

                            row.spawn((
                                StartReformButton(axis, delta_plus),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.3, 0.4, 1.0)),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(label_plus),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: FontSize::Px(10.0),
                                        ..default()
                                    },
                                ));
                            });
                        });
                    }
                });
        }

        parent.spawn((
            Text::new("[ Interest Groups ]"),
            TextColor(Color::srgb(0.9, 0.85, 0.6)),
            TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            },
        ));

        for ig_type in InterestGroupType::ALL {
            if let Some(ig_state) = pol.interest_groups.get(&ig_type) {
                parent
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        padding: UiRect::all(Val::Px(4.0)),
                        margin: UiRect::bottom(Val::Px(2.0)),
                        ..default()
                    })
                    .insert(BackgroundColor(Color::srgba(0.15, 0.15, 0.2, 0.9)))
                    .with_children(|row| {
                        row.spawn((
                            Text::new(format!(
                                "{:11} | Influence: {:4.1}% | Approval: {:+5.1} | Stance: {:+4.1}",
                                ig_type.display_name(),
                                ig_state.influence,
                                ig_state.approval,
                                ig_state.support_for_current_reform
                            )),
                            TextColor(if ig_state.approval > 20.0 {
                                Color::srgb(0.6, 0.9, 0.6)
                            } else if ig_state.approval < -20.0 {
                                Color::srgb(0.9, 0.6, 0.6)
                            } else {
                                Color::WHITE
                            }),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
            }
        }
    });
}
