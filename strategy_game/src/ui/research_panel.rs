use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::research::allocation::InProgressTech;
use crate::research::data::{TechnologyField, TechnologyRegistry};
use crate::research::world_stage::WorldCivilizationState;
use crate::ui::tab_bar::{TabBarRoot, ctrl_held};
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
            .add_systems(
                OnEnter(GameState::Playing),
                setup_research_panel.after(crate::ui::tab_bar::spawn_tab_bar),
            )
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

fn setup_research_panel(
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
                    ToggleResearchPanelButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.2, 0.3, 0.5, 0.9)),
                ))
                .with_children(|btn| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "research_panel.toggle_button", vec![]);
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

    // メインパネル（初期は非表示）
    // タイトルはスクロール対象の外(常に表示)、それ以外はスクロール可能な本体
    // (`ui::scroll::spawn_scrollable_body`)へ入れる(`ui::scroll`ドキュメント参照:
    // スクロールバーをスクロール対象の内側に置くと、つまみ自身も一緒にスクロールされ
    // 見かけ上「動かない」ように見える不具合が実機で確認されているため)。
    let research_panel_entity = commands
        .spawn((
            ResearchPanelRoot,
            // P21-013: 背景自体もButton化し、子Button以外の余白をクリック/ホバーしても
            // `Interaction`を確実に発行させる(`ui::load_confirm`の既存パターンを踏襲)。
            Button,
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
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.08, 0.12, 0.95)),
        ))
        .id();
    commands
        .entity(research_panel_entity)
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "research_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.9, 0.85, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            crate::ui::scroll::spawn_scrollable_body(parent, Val::Px(8.0), |parent| {
                parent.spawn((
                    ResearchHeaderText,
                    Text::new(""),
                    LocalizedText::default(),
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
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "research_panel.alloc_label",
                            vec![],
                        );
                        row.spawn((
                            text,
                            marker,
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
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "research_panel.alloc_field_button",
                                    vec![("field", t(&catalog, locale.0, field.display_name()))],
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
                                    let (text, marker) = localized_text(
                                        &catalog,
                                        locale.0,
                                        field.display_name(),
                                        vec![],
                                    );
                                    btn.spawn((
                                        text,
                                        marker,
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
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
            });
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
    // Ctrl+1: タブ切替は全パネル共通でCtrl+数字に統一
    // (軍事パネル内の素のDigit1/2/3/7/8/9フロントライン操作キーとの衝突を避けるため)。
    if keys.just_pressed(KeyCode::Digit1) && ctrl_held(&keys) {
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
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut header_q: Query<(&mut Text, &mut LocalizedText), With<ResearchHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<TechListContainer>>,
) {
    if !state.open {
        return;
    }

    let Ok((mut header_text, mut header_marker)) = header_q.single_mut() else {
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

    let header_args = vec![
        (
            "era",
            t(&catalog, locale.0, world_state.current_stage.display_name()),
        ),
        (
            "sci_cap",
            format!("{:.1}", country.science_research_capacity),
        ),
        ("mag_cap", format!("{:.1}", country.magic_research_capacity)),
        ("sci", format!("{:.0}", alloc.science * 100.0)),
        ("mag", format!("{:.0}", alloc.magic * 100.0)),
        ("mil", format!("{:.0}", alloc.military * 100.0)),
        ("fus", format!("{:.0}", alloc.fusion * 100.0)),
    ];
    let header_info = tf(
        &catalog,
        locale.0,
        "research_panel.header",
        header_args.clone(),
    );
    if header_text.0 != header_info {
        *header_text = Text::new(header_info);
    }
    header_marker.key = "research_panel.header";
    header_marker.args = header_args;

    if let Some(children) = children_opt {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    commands.entity(container_entity).with_children(|parent| {
        if let Some(in_prog) = country.research_state.in_progress.get(&active_field) {
            let tech_name = tech_registry
                .get(&in_prog.tech_id)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));

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
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "research_panel.researching",
                        vec![
                            ("tech", tech_name),
                            ("progress", format!("{:.0}", in_prog.progress)),
                            ("cost", format!("{:.0}", in_prog.cost)),
                        ],
                    );
                    row.spawn((
                        text,
                        marker,
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
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "research_panel.cancel_button",
                            vec![],
                        );
                        btn.spawn((
                            text,
                            marker,
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

                let status_key = if is_done {
                    "research_panel.status_completed"
                } else if is_in_prog {
                    "research_panel.status_in_progress"
                } else if !era_req_met {
                    "research_panel.status_locked_era"
                } else if !prereq_met {
                    "research_panel.status_locked_prereq"
                } else {
                    "research_panel.status_available"
                };
                let status_label = t(&catalog, locale.0, status_key);

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
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "research_panel.tech_line",
                                vec![
                                    ("name", def.name.clone()),
                                    ("status", status_label),
                                    ("cost", format!("{:.0}", def.cost)),
                                ],
                            );
                            col.spawn((
                                text,
                                marker,
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
                                let (text, marker) = localized_text(
                                    &catalog,
                                    locale.0,
                                    "research_panel.start_button",
                                    vec![],
                                );
                                btn.spawn((
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
                    });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_bar::spawn_tab_bar;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ActivePanel::default());
        app.insert_resource(ResearchPanelState::default());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_systems(Update, toggle_research_panel_key);
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
    fn ctrl_plus_digit1_toggles_research_panel() {
        let mut app = build_test_app();
        press_ctrl_plus(&mut app, KeyCode::Digit1);

        assert!(app.world().resource::<ResearchPanelState>().open);
        assert_eq!(
            app.world().resource::<ActivePanel>().current,
            PanelKind::Research
        );
    }

    #[test]
    fn bare_digit1_alone_does_not_toggle_research_panel() {
        let mut app = build_test_app();
        press_bare(&mut app, KeyCode::Digit1);

        assert!(
            !app.world().resource::<ResearchPanelState>().open,
            "Digit1 without Ctrl must not open the Research panel \
             (reserved to avoid colliding with military_panel's frontline command keys)"
        );
    }

    #[test]
    fn toggle_button_is_spawned_as_a_child_of_the_shared_tab_bar() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, (spawn_tab_bar, setup_research_panel).chain());
        app.update();

        let tab_bar = app
            .world_mut()
            .query_filtered::<Entity, With<crate::ui::tab_bar::TabBarRoot>>()
            .single(app.world())
            .expect("TabBarRoot must be spawned");
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<ToggleResearchPanelButton>>()
            .single(app.world())
            .expect("ToggleResearchPanelButton must be spawned");

        assert_eq!(
            app.world().entity(button).get::<ChildOf>().map(|c| c.0),
            Some(tab_bar),
            "the research toggle button must be a child of the shared TabBarRoot"
        );
    }

    /// P21-013: `ResearchPanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button以外の余白をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。
    #[test]
    fn research_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_research_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<ResearchPanelRoot>>()
            .single(app.world())
            .expect("ResearchPanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "ResearchPanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }
}
