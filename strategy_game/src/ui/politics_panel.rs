use crate::app::game_state::GameState;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::politics::interest_groups::InterestGroupType;
use crate::politics::reform::PoliticalReform;
use crate::politics::values::ValueAxis;
use crate::ui::tab_bar::{TabBarRoot, ctrl_held};
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
            .add_systems(
                OnEnter(GameState::Playing),
                setup_politics_panel.after(crate::ui::tab_bar::spawn_tab_bar),
            )
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

fn setup_politics_panel(
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
                    TogglePoliticsPanelButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.5, 0.2, 0.3, 0.9)),
                ))
                .with_children(|btn| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "politics_panel.toggle_button", vec![]);
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
    // (`ui::scroll::spawn_scrollable_body`)へ入れる(詳細は`ui::scroll`のドキュメント参照)。
    let politics_panel_entity = commands
        .spawn((
            PoliticsPanelRoot,
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
            BackgroundColor(Color::srgba(0.12, 0.08, 0.1, 0.95)),
        ))
        .id();
    commands
        .entity(politics_panel_entity)
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "politics_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.9, 0.7, 0.5)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            crate::ui::scroll::spawn_scrollable_body(parent, Val::Px(8.0), |parent| {
                parent.spawn((
                    PoliticsHeaderText,
                    Text::new(""),
                    LocalizedText::default(),
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
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
            });
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
    // Ctrl+2: タブ切替は全パネル共通でCtrl+数字に統一
    // (軍事パネル内の素のDigit1/2/3/7/8/9フロントライン操作キーとの衝突を避けるため)。
    if keys.just_pressed(KeyCode::Digit2) && ctrl_held(&keys) {
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

#[allow(clippy::too_many_arguments)]
fn update_politics_panel_ui(
    mut commands: Commands,
    state: Res<PoliticsPanelState>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut header_q: Query<(&mut Text, &mut LocalizedText), With<PoliticsHeaderText>>,
    container_q: Query<(Entity, Option<&Children>), With<PoliticsListContainer>>,
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

    let pol = &country.politics;
    let vals = &pol.values;

    let header_args = vec![
        (
            "government",
            t(&catalog, locale.0, country.government_type.display_name()),
        ),
        (
            "economy",
            t(&catalog, locale.0, country.economic_system.display_name()),
        ),
        ("sci_mag", format!("{:.0}", vals.science_magic)),
        ("ind_state", format!("{:.0}", vals.individual_state)),
        ("sec_rel", format!("{:.0}", vals.secular_religious)),
    ];
    let header_info = tf(
        &catalog,
        locale.0,
        "politics_panel.header",
        header_args.clone(),
    );
    if header_text.0 != header_info {
        *header_text = Text::new(header_info);
    }
    header_marker.key = "politics_panel.header";
    header_marker.args = header_args;

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
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "politics_panel.reform_line",
                        vec![
                            ("axis", t(&catalog, locale.0, reform.axis.display_name())),
                            ("target", format!("{:.0}", reform.target_value)),
                            ("progress", format!("{:.0}", reform.progress)),
                            ("required", format!("{:.0}", reform.required_progress)),
                            ("monthly", format!("{:.1}", reform.monthly_progress)),
                            ("resistance", format!("{:.1}", reform.clergy_resistance)),
                        ],
                    );
                    row.spawn((
                        text,
                        marker,
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
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "politics_panel.cancel_button",
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
        } else {
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(3.0),
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|col| {
                    let (text, marker) = localized_text(
                        &catalog,
                        locale.0,
                        "politics_panel.start_reform_header",
                        vec![],
                    );
                    col.spawn((
                        text,
                        marker,
                        TextColor(Color::srgb(0.9, 0.85, 0.6)),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                    ));

                    let axes = [
                        (
                            ValueAxis::ScienceMagic,
                            "politics_panel.axis_science_magic_minus",
                            -10.0,
                            "politics_panel.axis_science_magic_plus",
                            10.0,
                        ),
                        (
                            ValueAxis::IndividualState,
                            "politics_panel.axis_individual_state_minus",
                            -10.0,
                            "politics_panel.axis_individual_state_plus",
                            10.0,
                        ),
                        (
                            ValueAxis::SecularReligious,
                            "politics_panel.axis_secular_religious_minus",
                            -10.0,
                            "politics_panel.axis_secular_religious_plus",
                            10.0,
                        ),
                    ];

                    for (axis, label_minus_key, delta_minus, label_plus_key, delta_plus) in axes {
                        col.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            let (text, marker) = localized_text(
                                &catalog,
                                locale.0,
                                "politics_panel.axis_label",
                                vec![("axis", t(&catalog, locale.0, axis.display_name()))],
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
                                StartReformButton(axis, delta_minus),
                                Button,
                                Node {
                                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.25, 0.3, 0.4, 1.0)),
                            ))
                            .with_children(|b| {
                                let (text, marker) =
                                    localized_text(&catalog, locale.0, label_minus_key, vec![]);
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
                                let (text, marker) =
                                    localized_text(&catalog, locale.0, label_plus_key, vec![]);
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
                });
        }

        let (text, marker) = localized_text(
            &catalog,
            locale.0,
            "politics_panel.interest_groups_header",
            vec![],
        );
        parent.spawn((
            text,
            marker,
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
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "politics_panel.interest_group_line",
                            vec![
                                ("group", t(&catalog, locale.0, ig_type.display_name())),
                                ("influence", format!("{:.1}", ig_state.influence)),
                                ("approval", format!("{:+.1}", ig_state.approval)),
                                (
                                    "stance",
                                    format!("{:+.1}", ig_state.support_for_current_reform),
                                ),
                            ],
                        );
                        row.spawn((
                            text,
                            marker,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_bar::spawn_tab_bar;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ActivePanel::default());
        app.insert_resource(PoliticsPanelState::default());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_systems(Update, toggle_politics_panel_key);
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
    fn ctrl_plus_digit2_toggles_politics_panel() {
        let mut app = build_test_app();
        press_ctrl_plus(&mut app, KeyCode::Digit2);

        assert!(app.world().resource::<PoliticsPanelState>().open);
        assert_eq!(
            app.world().resource::<ActivePanel>().current,
            PanelKind::Politics
        );
    }

    #[test]
    fn bare_digit2_alone_does_not_toggle_politics_panel() {
        let mut app = build_test_app();
        press_bare(&mut app, KeyCode::Digit2);

        assert!(
            !app.world().resource::<PoliticsPanelState>().open,
            "Digit2 without Ctrl must not open the Politics panel"
        );
    }

    #[test]
    fn toggle_button_is_spawned_as_a_child_of_the_shared_tab_bar() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, (spawn_tab_bar, setup_politics_panel).chain());
        app.update();

        let tab_bar = app
            .world_mut()
            .query_filtered::<Entity, With<crate::ui::tab_bar::TabBarRoot>>()
            .single(app.world())
            .expect("TabBarRoot must be spawned");
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<TogglePoliticsPanelButton>>()
            .single(app.world())
            .expect("TogglePoliticsPanelButton must be spawned");

        assert_eq!(
            app.world().entity(button).get::<ChildOf>().map(|c| c.0),
            Some(tab_bar),
            "the politics toggle button must be a child of the shared TabBarRoot"
        );
    }

    /// P21-013: `PoliticsPanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button以外の余白をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。
    #[test]
    fn politics_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_politics_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<PoliticsPanelRoot>>()
            .single(app.world())
            .expect("PoliticsPanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "PoliticsPanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }
}
