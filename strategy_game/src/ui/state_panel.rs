use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::building::construction::{ConstructionQueueItem, ConstructionStatus, REFUND_RATIO};
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::economy::resources::{ResourceType, has_discovered_deposit, is_crystal_only_state};
use crate::localization::{
    CurrentLocale, LocalizedText, TranslationCatalog, localized_text, t, tf,
};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use bevy::prelude::*;

#[derive(Component)]
pub struct StatePanelRoot;

#[derive(Component)]
pub struct StatePanelText;

#[derive(Component)]
pub struct BuildButton(pub BuildingType);

pub struct StatePanelPlugin;

impl Plugin for StatePanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_state_panel)
            .add_systems(
                Update,
                (
                    handle_build_buttons.run_if(crate::country::player_can_issue_commands),
                    handle_cancel_queue_buttons.run_if(crate::country::player_can_issue_commands),
                )
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                // P23-003: 州人口・労働力・動員負担の表示は、月次経済処理(Economy Set)が
                // 確定させた`employed_workforce`/`unemployed_workforce`を読む。明示的なSet
                // 制約がないと同一フレーム内でEconomyより先に実行され得て、月替わり直後の
                // 1フレームだけ表示が古い値になる(本タスクで実際に統合テストが検出した)。
                update_state_panel
                    .in_set(DailySimulationSet::UiUpdate)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_state_panel(
    mut commands: Commands,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    commands
        .spawn((
            StatePanelRoot,
            // P21-013: 背景自体もButton化し、子Button以外の余白をクリック/ホバーしても
            // `Interaction`を確実に発行させる(`ui::load_confirm`の既存パターンを踏襲)。
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(40.0),
                bottom: Val::Px(0.0),
                width: Val::Px(340.0),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.9)),
        ))
        .with_children(|parent| {
            let (text, marker) = localized_text(&catalog, locale.0, "state_panel.title", vec![]);
            parent.spawn((
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.85, 0.6)),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
            ));

            let (text, marker) =
                localized_text(&catalog, locale.0, "state_panel.select_prompt", vec![]);
            parent.spawn((
                StatePanelText,
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            let (text, marker) =
                localized_text(&catalog, locale.0, "state_panel.buildings_header", vec![]);
            parent.spawn((
                text,
                marker,
                TextLayout {
                    linebreak: LineBreak::AnyCharacter,
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.9, 0.7)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            for b_type in BuildingType::ALL {
                parent
                    .spawn((
                        BuildButton(b_type),
                        Button,
                        Node {
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(4.0)),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            margin: UiRect::bottom(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.2, 0.25, 0.3, 1.0)),
                    ))
                    .with_children(|btn| {
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "state_panel.build_button",
                            vec![("building", t(&catalog, locale.0, b_type.display_name()))],
                        );
                        btn.spawn((
                            text,
                            marker,
                            TextLayout {
                                linebreak: LineBreak::AnyCharacter,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                        ));
                    });
            }
        });
}

fn update_state_panel(
    selected: Res<SelectedState>,
    state_registry: Res<StateRegistry>,
    country_registry: Res<CountryRegistry>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut text_q: Query<(&mut Text, &mut LocalizedText), With<StatePanelText>>,
) {
    let Ok((mut text, mut marker)) = text_q.single_mut() else {
        return;
    };

    let Some(state_id) = selected.0 else {
        let prompt_key = "state_panel.select_prompt";
        let rendered = t(&catalog, locale.0, prompt_key);
        if text.0 != rendered {
            *text = Text::new(rendered);
        }
        marker.key = prompt_key;
        marker.args = vec![];
        return;
    };

    let Some(state) = state_registry.get(state_id) else {
        return;
    };

    let country_name = country_registry
        .get(state.owner_country_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));

    let none_line = t(&catalog, locale.0, "state_panel.none_line");

    let mut b_str = String::new();
    for b_type in BuildingType::ALL {
        let lvl = state.building_level(b_type);
        if lvl > 0 {
            let op = state.building_operation(b_type);
            b_str.push_str(&tf(
                &catalog,
                locale.0,
                "state_panel.building_line",
                vec![
                    ("building", t(&catalog, locale.0, b_type.display_name())),
                    ("level", lvl.to_string()),
                    ("operation", format!("{:.0}", op * 100.0)),
                ],
            ));
        }
    }
    if b_str.is_empty() {
        b_str.push_str(&none_line);
    }

    let mut dep_str = String::new();
    for dep in &state.resource_deposits {
        if dep.discovered {
            dep_str.push_str(&tf(
                &catalog,
                locale.0,
                "state_panel.deposit_line",
                vec![
                    (
                        "resource",
                        t(&catalog, locale.0, dep.resource_type.display_name()),
                    ),
                    ("output", format!("{:.0}", dep.base_output)),
                ],
            ));
        }
    }
    if dep_str.is_empty() {
        dep_str.push_str(&none_line);
    }

    // P23-003: 表示する労働力は動員負担控除後の民間労働力(employed+unemployed、月次
    // 雇用計算が確定させた実値)を使う。total_workforce()(人口とworkforce_ratioから
    // 動員を考慮せず再導出する潜在労働力)は使わない——動員中はこの2つが乖離する。
    let civilian_wf = state.civilian_workforce();
    let employment_rate =
        crate::population::update::calculate_employment_rate(state.employed_workforce, civilian_wf);

    // 州の所有国の`mobilized_manpower`を、現在の所有州人口比で按分した結果のうち
    // この州の分だけを表示する。所有国が見つからない(異常系)場合は0扱い。
    // P23-002の既存按分関数(`apportion_mobilized_manpower`)をそのまま再利用し、
    // UI側で最大剰余法を複製しない。
    let mobilized_burden = country_registry
        .get(state.owner_country_id)
        .map(|owner| {
            let owner_states: Vec<(crate::common::StateId, u64)> = state_registry
                .states
                .iter()
                .filter(|s| s.owner_country_id == state.owner_country_id)
                .map(|s| (s.id, s.population))
                .collect();
            crate::population::mobilization::apportion_mobilized_manpower(
                owner.mobilized_manpower,
                &owner_states,
            )
            .get(&state.id)
            .copied()
            .unwrap_or(0)
        })
        .unwrap_or(0);

    let args = vec![
        ("name", state.name.clone()),
        ("owner", country_name),
        ("population", format_population(state.population)),
        ("workforce", format_population(civilian_wf)),
        ("employed", format_population(state.employed_workforce)),
        ("unemployed", format_population(state.unemployed_workforce)),
        ("employment_rate", format!("{:.0}", employment_rate * 100.0)),
        ("mobilized_burden", format_population(mobilized_burden)),
        ("living_standard", format!("{:.1}", state.living_standard)),
        ("unrest", format!("{:.1}", state.unrest)),
        ("logistics_cap", format!("{:.0}", state.logistics_capacity)),
        ("logistics_usage", format!("{:.0}", state.logistics_usage)),
        (
            "logistics_ratio",
            format!("{:.0}", state.logistics_ratio * 100.0),
        ),
        ("deposits", dep_str),
        ("buildings", b_str),
    ];
    let info = tf(&catalog, locale.0, "state_panel.info", args.clone());

    if text.0 != info {
        *text = Text::new(info);
    }
    marker.key = "state_panel.info";
    marker.args = args;
}

#[allow(clippy::too_many_arguments)]
fn handle_build_buttons(
    mut interaction_q: Query<
        (&Interaction, &BuildButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    selected: Res<SelectedState>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    building_registry: Res<BuildingRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    let Some(state_id) = selected.0 else {
        return;
    };

    let Some(player_cid) = player_country.0 else {
        return;
    };

    // P21-021: run_ifとは独立に、Domain APIレベルでも消滅済み国家の建設命令を拒否する。
    if country_registry
        .get(player_cid)
        .is_none_or(|c| !c.is_active())
    {
        return;
    }

    let Some(state) = state_registry.get(state_id) else {
        return;
    };

    for (interaction, btn, mut bg) in interaction_q.iter_mut() {
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.4, 0.5, 0.6, 1.0));

                if state.owner_country_id != player_cid {
                    notif_writer.write(GameNotification {
                        message: t(&catalog, locale.0, "state_panel.build_failed_not_owner"),
                    });
                    continue;
                }

                let def = match building_registry.get(btn.0) {
                    Some(d) => d,
                    None => continue,
                };

                if btn.0.requires_magic_crystal_deposit()
                    && !has_discovered_deposit(&state.resource_deposits, ResourceType::MagicCrystal)
                {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "state_panel.build_failed_no_deposit",
                            vec![
                                ("building", t(&catalog, locale.0, btn.0.display_name())),
                                ("state", state.name.clone()),
                            ],
                        ),
                    });
                    continue;
                }

                // P21-009-FIX-001: クリスタル専用州(MagicCrystal鉱床のみ)には通常Mineの
                // 対象鉱床が存在しないため、新規建設を許可しない(CrystalMineを使う)。
                if btn.0 == BuildingType::Mine && is_crystal_only_state(&state.resource_deposits) {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "state_panel.build_failed_crystal_only_state",
                            vec![
                                ("building", t(&catalog, locale.0, btn.0.display_name())),
                                ("state", state.name.clone()),
                            ],
                        ),
                    });
                    continue;
                }

                let current_level = state.building_level(btn.0);
                let country = match country_registry.get_mut(player_cid) {
                    Some(c) => c,
                    None => continue,
                };

                if current_level >= def.max_level {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "state_panel.build_failed_max_level",
                            vec![
                                ("building", t(&catalog, locale.0, btn.0.display_name())),
                                ("max_level", def.max_level.to_string()),
                            ],
                        ),
                    });
                    continue;
                }

                let in_queue = country
                    .construction_queue
                    .iter()
                    .any(|item| item.state_id == state_id && item.building_type == btn.0);
                if in_queue {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "state_panel.build_failed_in_queue",
                            vec![
                                ("building", t(&catalog, locale.0, btn.0.display_name())),
                                ("state", state.name.clone()),
                            ],
                        ),
                    });
                    continue;
                }

                if country.treasury < def.construction_cost {
                    notif_writer.write(GameNotification {
                        message: tf(
                            &catalog,
                            locale.0,
                            "state_panel.build_failed_funds",
                            vec![
                                ("cost", format!("{:.0}", def.construction_cost)),
                                ("treasury", format!("{:.0}", country.treasury)),
                            ],
                        ),
                    });
                    continue;
                }

                country.treasury -= def.construction_cost;
                country.construction_queue.push(ConstructionQueueItem {
                    state_id,
                    building_type: btn.0,
                    target_level: current_level + 1,
                    progress: 0.0,
                    required_progress: def.required_progress,
                    paid_cost: def.construction_cost,
                    status: ConstructionStatus::InQueue,
                });

                notif_writer.write(GameNotification {
                    message: tf(
                        &catalog,
                        locale.0,
                        "state_panel.build_started",
                        vec![
                            ("building", t(&catalog, locale.0, btn.0.display_name())),
                            ("state", state.name.clone()),
                            ("level", (current_level + 1).to_string()),
                        ],
                    ),
                });
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.3, 0.35, 0.45, 1.0));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.2, 0.25, 0.3, 1.0));
            }
        }
    }
}

fn handle_cancel_queue_buttons(
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
    keys: Res<ButtonInput<KeyCode>>,
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        let Some(player_cid) = player_country.0 else {
            return;
        };
        let Some(country) = country_registry.get_mut(player_cid) else {
            return;
        };
        // P21-021: run_ifとは独立に、Domain APIレベルでも消滅済み国家の建設キャンセルを拒否する。
        if !country.is_active() {
            return;
        }

        if let Some(item) = country.construction_queue.pop() {
            let refund = item.paid_cost * REFUND_RATIO;
            country.treasury += refund;

            notif_writer.write(GameNotification {
                message: tf(
                    &catalog,
                    locale.0,
                    "state_panel.build_cancelled",
                    vec![
                        (
                            "building",
                            t(&catalog, locale.0, item.building_type.display_name()),
                        ),
                        ("refund", format!("{:.0}", refund)),
                    ],
                ),
            });
        }
    }
}

pub fn format_population(pop: u64) -> String {
    if pop >= 1_000_000 {
        format!("{:.2}M", pop as f64 / 1_000_000.0)
    } else if pop >= 1_000 {
        format!("{:.1}K", pop as f64 / 1_000.0)
    } else {
        format!("{}", pop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, StateId};
    use crate::country::CountryData;
    use crate::economy::resources::StateResourceDeposit;
    use crate::state::data::{StateData, StateRegistry};
    use bevy::ecs::system::SystemState;
    use std::collections::HashMap;

    fn building_def(building_type: BuildingType) -> crate::building::data::BuildingDefinition {
        crate::building::data::BuildingDefinition {
            building_type,
            name: format!("{building_type:?}"),
            construction_cost: 100.0,
            required_progress: 10.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::new(),
            maintenance_cost: 1.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn build_test_app(state: StateData) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<GameNotification>();
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Update, handle_build_buttons);

        let mut definitions = HashMap::new();
        definitions.insert(BuildingType::Mine, building_def(BuildingType::Mine));
        definitions.insert(
            BuildingType::CrystalMine,
            building_def(BuildingType::CrystalMine),
        );
        definitions.insert(
            BuildingType::CrystalRefinery,
            building_def(BuildingType::CrystalRefinery),
        );
        app.insert_resource(BuildingRegistry { definitions });

        app.insert_resource(SelectedState(Some(state.id)));
        app.insert_resource(PlayerCountry(Some(state.owner_country_id)));
        app.insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: state.owner_country_id,
                treasury: 10_000.0,
                ..CountryData::default()
            }],
        });
        app.insert_resource(StateRegistry::build(vec![state]));
        app
    }

    fn press_build_button(app: &mut App, building_type: BuildingType) {
        app.world_mut().spawn((
            BuildButton(building_type),
            Interaction::Pressed,
            BackgroundColor(Color::WHITE),
        ));
        app.update();
    }

    fn read_notifications(app: &mut App) -> Vec<String> {
        let mut state: SystemState<MessageReader<GameNotification>> =
            SystemState::new(app.world_mut());
        state
            .get_mut(app.world_mut())
            .expect("reader")
            .read()
            .map(|n| n.message.clone())
            .collect()
    }

    /// P21-013: `StatePanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button(建設ボタン等)以外の余白をクリック/ホバーしても`Interaction`が確実に
    /// 発行され、`map::selection::handle_state_click`等の既存「UIのHovered/Pressed中は
    /// マップ操作をスキップする」ガードがこの領域にも効くようになる
    /// (`ui::load_confirm`の既存パターンと同じ)。
    #[test]
    fn state_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_state_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<StatePanelRoot>>()
            .single(app.world())
            .expect("StatePanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "StatePanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }

    fn state_with_deposit(id: StateId, owner: CountryId, has_deposit: bool) -> StateData {
        StateData {
            id,
            owner_country_id: owner,
            population: 100_000,
            resource_deposits: if has_deposit {
                vec![StateResourceDeposit {
                    resource_type: ResourceType::MagicCrystal,
                    base_output: 30.0,
                    discovered: true,
                    development_level: 1,
                }]
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    /// 要求テスト項目15/16: 建設UIからクリスタル採掘施設・精製施設の建設をリクエストできる。
    #[test]
    fn crystal_mine_build_request_succeeds_when_state_has_deposit() {
        let state = state_with_deposit(StateId(0), CountryId(0), true);
        let mut app = build_test_app(state);

        press_build_button(&mut app, BuildingType::CrystalMine);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert_eq!(country_registry.countries[0].construction_queue.len(), 1);
        assert_eq!(
            country_registry.countries[0].construction_queue[0].building_type,
            BuildingType::CrystalMine
        );
    }

    /// P21-021: 消滅済み国家はDomain APIレベルで建設命令を拒否される
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn handle_build_buttons_rejects_an_eliminated_country() {
        let state = state_with_deposit(StateId(0), CountryId(0), false);
        let mut app = build_test_app(state);
        app.world_mut()
            .resource_mut::<CountryRegistry>()
            .get_mut(CountryId(0))
            .unwrap()
            .status = crate::country::CountryStatus::Eliminated {
            eliminated_date: "1936/01/01".to_string(),
            annexed_by: CountryId(9),
        };

        press_build_button(&mut app, BuildingType::CrystalRefinery);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert_eq!(
            country_registry.countries[0].construction_queue.len(),
            0,
            "an eliminated country must not be able to start construction"
        );
    }

    #[test]
    fn crystal_refinery_build_request_succeeds_without_deposit() {
        let state = state_with_deposit(StateId(0), CountryId(0), false);
        let mut app = build_test_app(state);

        press_build_button(&mut app, BuildingType::CrystalRefinery);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert_eq!(
            country_registry.countries[0].construction_queue.len(),
            1,
            "CrystalRefinery must be buildable without a magic crystal deposit, like Factory"
        );
    }

    /// 要求テスト項目17: 鉱床がない州では採掘不可理由が通知として表示される。
    #[test]
    fn crystal_mine_build_blocked_without_deposit_shows_reason() {
        let state = state_with_deposit(StateId(0), CountryId(0), false);
        let mut app = build_test_app(state);

        press_build_button(&mut app, BuildingType::CrystalMine);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert!(
            country_registry.countries[0].construction_queue.is_empty(),
            "no deposit must block the build request"
        );

        let notifications = read_notifications(&mut app);
        assert_eq!(notifications.len(), 1);
        let catalog = app.world().resource::<TranslationCatalog>();
        let locale = app.world().resource::<CurrentLocale>();
        let expected = tf(
            catalog,
            locale.0,
            "state_panel.build_failed_no_deposit",
            vec![
                (
                    "building",
                    t(catalog, locale.0, BuildingType::CrystalMine.display_name()),
                ),
                ("state", "Default State".to_string()),
            ],
        );
        assert_eq!(
            notifications[0], expected,
            "reason notification must be shown, got: {notifications:?}"
        );
    }

    /// P21-009-FIX-001要求テスト項目7: クリスタル専用州へ通常Mineを新規建設できない。
    #[test]
    fn mine_build_blocked_in_crystal_only_state_shows_reason() {
        let state = state_with_deposit(StateId(0), CountryId(0), true); // MagicCrystal専用
        let mut app = build_test_app(state);

        press_build_button(&mut app, BuildingType::Mine);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert!(
            country_registry.countries[0].construction_queue.is_empty(),
            "crystal-only state must block new ordinary Mine construction"
        );

        let notifications = read_notifications(&mut app);
        assert_eq!(notifications.len(), 1);
        let catalog = app.world().resource::<TranslationCatalog>();
        let locale = app.world().resource::<CurrentLocale>();
        let expected = tf(
            catalog,
            locale.0,
            "state_panel.build_failed_crystal_only_state",
            vec![
                (
                    "building",
                    t(catalog, locale.0, BuildingType::Mine.display_name()),
                ),
                ("state", "Default State".to_string()),
            ],
        );
        assert_eq!(notifications[0], expected);
    }

    /// Iron等の非クリスタル州では、従来どおり通常Mineを建設できる(回帰防止)。
    #[test]
    fn mine_build_still_succeeds_in_non_crystal_state() {
        let mut state = state_with_deposit(StateId(0), CountryId(0), false);
        state.resource_deposits = vec![StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }];
        let mut app = build_test_app(state);

        press_build_button(&mut app, BuildingType::Mine);

        let country_registry = app.world().resource::<CountryRegistry>();
        assert_eq!(
            country_registry.countries[0].construction_queue.len(),
            1,
            "Iron-deposit state must still allow ordinary Mine construction"
        );
        assert_eq!(
            country_registry.countries[0].construction_queue[0].building_type,
            BuildingType::Mine
        );
    }

    /// P21-021: 消滅済み国家はDomain APIレベルで建設キャンセル命令も拒否される
    /// (UIのrun_ifだけに依存しない直接検証)。
    #[test]
    fn handle_cancel_queue_buttons_rejects_an_eliminated_country() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<GameNotification>();
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Update, handle_cancel_queue_buttons);
        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_resource(CountryRegistry {
            countries: vec![CountryData {
                id: CountryId(0),
                treasury: 0.0,
                construction_queue: vec![ConstructionQueueItem {
                    state_id: StateId(0),
                    building_type: BuildingType::Mine,
                    target_level: 1,
                    progress: 0.0,
                    required_progress: 10.0,
                    paid_cost: 100.0,
                    status: ConstructionStatus::InQueue,
                }],
                status: crate::country::CountryStatus::Eliminated {
                    eliminated_date: "1936/01/01".to_string(),
                    annexed_by: CountryId(9),
                },
                ..CountryData::default()
            }],
        });

        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyC);
        app.insert_resource(keys);

        app.update();

        let country_registry = app.world().resource::<CountryRegistry>();
        assert_eq!(
            country_registry.countries[0].construction_queue.len(),
            1,
            "an eliminated country's queue must survive rejection"
        );
        assert_eq!(country_registry.countries[0].treasury, 0.0);
    }

    // ─── P23-003: 人口・人的資源・動員負担のUI表示 ─────────────────────────────

    fn build_state_panel_display_app(states: Vec<StateData>, countries: Vec<CountryData>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Update, update_state_panel);
        let selected = states.first().map(|s| s.id);
        app.insert_resource(SelectedState(selected));
        app.insert_resource(CountryRegistry { countries });
        app.insert_resource(StateRegistry::build(states));
        app.world_mut().spawn((
            StatePanelText,
            Text::new(String::new()),
            LocalizedText::default(),
        ));
        app
    }

    fn read_panel_args(app: &mut App) -> Vec<(&'static str, String)> {
        app.world_mut()
            .query_filtered::<&LocalizedText, With<StatePanelText>>()
            .single(app.world())
            .expect("StatePanelText must be spawned")
            .args
            .clone()
    }

    fn panel_arg<'a>(args: &'a [(&'static str, String)], key: &str) -> &'a str {
        args.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("expected arg '{key}' to be present, got: {args:?}"))
    }

    fn base_state(id: StateId, owner: CountryId) -> StateData {
        StateData {
            id,
            owner_country_id: owner,
            population: 100_000,
            workforce_ratio: 0.5, // total_workforce()なら50,000だが、下記テストは動員込みの実値を使う
            ..Default::default()
        }
    }

    fn base_country(id: CountryId, mobilized_manpower: u64) -> CountryData {
        CountryData {
            id,
            mobilized_manpower,
            ..CountryData::default()
        }
    }

    /// 要求テスト項目1/2: 動員負担控除後の民間労働力(employed+unemployed)を表示し、
    /// total_workforce()(人口*workforce_ratioの潜在労働力、動員を考慮しない)は表示しない。
    #[test]
    fn workforce_display_uses_civilian_workforce_not_total_workforce() {
        let mut state = base_state(StateId(0), CountryId(0));
        state.employed_workforce = 20_000;
        state.unemployed_workforce = 10_000; // civilian_workforce=30,000 != total_workforce()=50,000
        let mut app =
            build_state_panel_display_app(vec![state], vec![base_country(CountryId(0), 0)]);
        app.update();

        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "workforce"), format_population(30_000));
    }

    /// 要求テスト項目3: 雇用率は動員控除後の民間労働力を分母にする。
    #[test]
    fn employment_rate_uses_employed_over_civilian_workforce() {
        let mut state = base_state(StateId(0), CountryId(0));
        state.employed_workforce = 25_000;
        state.unemployed_workforce = 25_000; // civilian_workforce=50,000 -> rate 50%
        let mut app =
            build_state_panel_display_app(vec![state], vec![base_country(CountryId(0), 0)]);
        app.update();

        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "employment_rate"), "50");
    }

    /// 要求テスト項目4: 民間労働力0(人口0州等)でも0除算・NaN・panicが起きない。
    #[test]
    fn employment_rate_is_zero_when_civilian_workforce_is_zero() {
        let mut state = base_state(StateId(0), CountryId(0));
        state.population = 0;
        state.employed_workforce = 0;
        state.unemployed_workforce = 0;
        let mut app =
            build_state_panel_display_app(vec![state], vec![base_country(CountryId(0), 0)]);
        app.update();

        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "employment_rate"), "0");
        assert_eq!(panel_arg(&args, "workforce"), format_population(0));
    }

    /// 要求テスト項目5: 動員負担が既存の按分Domain API(`apportion_mobilized_manpower`)の
    /// 結果と一致する(UI側で最大剰余法を複製しない契約の確認)。
    #[test]
    fn mobilized_burden_matches_the_population_weighted_apportionment() {
        let state_a = base_state(StateId(0), CountryId(0)); // population 100,000
        let mut state_b = base_state(StateId(1), CountryId(0));
        state_b.population = 300_000; // 比率1:3
        let country = base_country(CountryId(0), 4_000);

        let expected = crate::population::mobilization::apportion_mobilized_manpower(
            4_000,
            &[(StateId(0), 100_000), (StateId(1), 300_000)],
        );

        let mut app = build_state_panel_display_app(vec![state_a, state_b], vec![country]);
        app.update();

        let args = read_panel_args(&mut app);
        let expected_burden = expected.get(&StateId(0)).copied().unwrap();
        assert_eq!(
            panel_arg(&args, "mobilized_burden"),
            format_population(expected_burden)
        );
        assert_eq!(expected_burden, 1_000);
    }

    /// 動員済み人的資源0の国では、動員負担表示は常に0。
    #[test]
    fn mobilized_burden_is_zero_when_country_has_no_mobilized_manpower() {
        let state = base_state(StateId(0), CountryId(0));
        let mut app =
            build_state_panel_display_app(vec![state], vec![base_country(CountryId(0), 0)]);
        app.update();

        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "mobilized_burden"), format_population(0));
    }

    /// 無所属州(所有国がCountryRegistryに存在しない)でも動員負担表示はpanicせず0になる。
    #[test]
    fn mobilized_burden_is_zero_for_a_state_whose_owner_country_is_missing() {
        let state = base_state(StateId(0), CountryId(99));
        let mut app = build_state_panel_display_app(vec![state], vec![]);
        app.update();

        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "mobilized_burden"), format_population(0));
    }

    /// 要求テスト項目3(UI統合): 州選択を変更すると表示値が切り替わる。
    #[test]
    fn state_panel_display_updates_when_the_selected_state_changes() {
        let mut state_a = base_state(StateId(0), CountryId(0));
        state_a.employed_workforce = 10_000;
        state_a.unemployed_workforce = 0;
        let mut state_b = base_state(StateId(1), CountryId(0));
        state_b.employed_workforce = 40_000;
        state_b.unemployed_workforce = 0;

        let mut app = build_state_panel_display_app(
            vec![state_a, state_b],
            vec![base_country(CountryId(0), 0)],
        );
        app.update();
        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "workforce"), format_population(10_000));

        app.insert_resource(SelectedState(Some(StateId(1))));
        app.update();
        let args = read_panel_args(&mut app);
        assert_eq!(panel_arg(&args, "workforce"), format_population(40_000));
    }
}
