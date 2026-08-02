use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::WarId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::capitulation::{
    check_attacker_capitulation, check_defender_capitulation, has_combat_ready_armies,
};
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::peace::{PeaceOffer, PeaceTerm, can_accept_peace_offer, execute_peace_settlement};
use crate::war::war_score::calculate_war_score;
use bevy::prelude::*;

#[derive(Component)]
pub struct PeacePanelRoot;

#[derive(Component)]
pub struct TogglePeacePanelButton;

#[derive(Component)]
pub struct PeacePanelContentContainer;

#[derive(Component)]
pub struct PeaceActionButton {
    pub war_id: WarId,
    pub term: PeaceTerm,
}

#[derive(Resource, Default)]
pub struct PeacePanelState {
    pub open: bool,
    pub selected_war_id: Option<WarId>,
}

pub struct PeacePanelPlugin;

impl Plugin for PeacePanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PeacePanelState::default())
            .add_systems(OnEnter(GameState::Playing), setup_peace_panel)
            .add_systems(
                Update,
                (
                    toggle_peace_panel_key,
                    handle_toggle_button,
                    handle_peace_action_buttons,
                    update_peace_panel_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_peace_panel(mut commands: Commands) {
    // [P] Peace Panel Button
    commands
        .spawn((
            TogglePeacePanelButton,
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(710.0),
                top: Val::Px(45.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.3, 0.2, 0.4, 0.9)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("[P] War & Peace Panel"),
                TextColor(Color::WHITE),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
        });

    // Peace Panel Window
    commands
        .spawn((
            PeacePanelRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(250.0),
                top: Val::Px(80.0),
                width: Val::Px(700.0),
                height: Val::Px(620.0),
                padding: UiRect::all(Val::Px(16.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                display: Display::None,
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.08, 0.12, 0.95)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("=== War Progress & Peace Conference ==="),
                TextColor(Color::srgb(0.9, 0.8, 0.4)),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
            ));

            parent.spawn((
                PeacePanelContentContainer,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    overflow: Overflow::clip_y(),
                    flex_grow: 1.0,
                    ..default()
                },
            ));
        });
}

fn toggle_peace_panel_key(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<PeacePanelState>) {
    if keys.just_pressed(KeyCode::KeyP) {
        state.open = !state.open;
    }
}

fn handle_toggle_button(
    mut state: ResMut<PeacePanelState>,
    q_btn: Query<&Interaction, (Changed<Interaction>, With<TogglePeacePanelButton>)>,
) {
    for interaction in q_btn.iter() {
        if *interaction == Interaction::Pressed {
            state.open = !state.open;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn handle_peace_action_buttons(
    q_btn: Query<(&Interaction, &PeaceActionButton), (Changed<Interaction>, With<Button>)>,
    player_country: Res<PlayerCountry>,
    game_date: Res<GameDate>,
    mut war_registry: ResMut<WarRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut battle_registry: ResMut<BattleRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    mut frontline_registry: ResMut<crate::war::frontline::FrontlineRegistry>,
    mut notif_writer: MessageWriter<GameNotification>,
) {
    let player_id = match player_country.0 {
        Some(id) => id,
        None => return,
    };
    let current_date_str = game_date.display();

    for (interaction, btn) in q_btn.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let war = match war_registry.wars.get(&btn.war_id) {
            Some(w) => w,
            None => continue,
        };

        let recipient_id = if war.attackers.contains(&player_id) {
            *war.defenders.iter().next().unwrap_or(&player_id)
        } else {
            *war.attackers.iter().next().unwrap_or(&player_id)
        };

        let offer = PeaceOffer {
            war_id: btn.war_id,
            proposer_country_id: player_id,
            recipient_country_id: recipient_id,
            term: btn.term,
            created_date: current_date_str.clone(),
        };

        // 講和条件の検証
        match can_accept_peace_offer(
            &offer,
            &war_registry,
            Some(&state_registry),
            Some(&military_registry),
            &current_date_str,
        ) {
            Ok(()) => {
                let reason = format!("Peace Treaty Accepted: {}", btn.term.display_name());
                let result = execute_peace_settlement(
                    btn.war_id,
                    btn.term,
                    &reason,
                    &current_date_str,
                    &mut state_registry,
                    &mut war_registry,
                    &mut military_registry,
                    &mut battle_registry,
                    &mut diplomacy_registry,
                    &mut frontline_registry,
                );

                if result.is_ok() {
                    notif_writer.write(GameNotification {
                        message: format!("Peace Enacted: {}", btn.term.display_name()),
                    });
                }
            }
            Err(err_msg) => {
                notif_writer.write(GameNotification {
                    message: format!("Peace Offer Rejected: {}", err_msg),
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_peace_panel_ui(
    panel_state: Res<PeacePanelState>,
    player_country: Res<PlayerCountry>,
    game_date: Res<GameDate>,
    war_registry: Res<WarRegistry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    diplomacy_registry: Res<DiplomacyRegistry>,
    mut q_panel: Query<&mut Node, With<PeacePanelRoot>>,
    q_container: Query<(Entity, Option<&Children>), With<PeacePanelContentContainer>>,
    mut commands: Commands,
) {
    let Ok(mut root_node) = q_panel.single_mut() else {
        return;
    };

    if !panel_state.open {
        root_node.display = Display::None;
        return;
    }

    root_node.display = Display::Flex;

    let Ok((container_entity, children_opt)) = q_container.single() else {
        return;
    };

    if let Some(children) = children_opt {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    let player_id = match player_country.0 {
        Some(id) => id,
        None => {
            commands.entity(container_entity).with_children(|parent| {
                parent.spawn((
                    Text::new("No player country selected."),
                    TextColor(Color::srgb(0.5, 0.5, 0.5)),
                ));
            });
            return;
        }
    };

    let current_date_str = game_date.display();

    // プレイヤーが関与するアクティブな戦争
    let active_war = war_registry.wars.values().find(|w| {
        w.status == WarStatus::Active
            && (w.attackers.contains(&player_id) || w.defenders.contains(&player_id))
    });

    commands.entity(container_entity).with_children(|parent| {
        if let Some(war) = active_war {
            let atk_id = *war.attackers.iter().next().unwrap_or(&player_id);
            let def_id = *war.defenders.iter().next().unwrap_or(&player_id);

            let atk_name = country_registry
                .get(atk_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Attacker".into());
            let def_name = country_registry
                .get(def_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Defender".into());

            let is_player_attacker = war.attackers.contains(&player_id);
            let role_str = if is_player_attacker {
                "Attacker"
            } else {
                "Defender"
            };

            let start_date = GameDate::from_string(&war.start_date).unwrap_or_default();
            let elapsed_days = game_date.days_since(&start_date).max(0);

            let breakdown = calculate_war_score(war, &state_registry);

            let atk_owned = state_registry.get_owned_states(atk_id);
            let def_owned = state_registry.get_owned_states(def_id);

            let atk_occ_def = def_owned.iter().filter(|s| s.controller() == atk_id).count();
            let def_occ_atk = atk_owned.iter().filter(|s| s.controller() == def_id).count();

            parent.spawn((
                Text::new(format!(
                    "Active War: {} (vs {}) [{}]\nDuration: {} days (Started: {})",
                    war.name,
                    if is_player_attacker { &def_name } else { &atk_name },
                    role_str,
                    elapsed_days,
                    war.start_date
                )),
                TextColor(Color::srgb(0.4, 0.9, 1.0)),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
            ));

            // 戦勝点内訳
            parent.spawn((
                Text::new(format!(
                    "War Score: {}\n - War Goal: +{}\n - Attacker Occupations: +{} ({}/{} states)\n - Defender Occupations: {} ({}/{} states)\n - Battle Wins: +{} (Attacker {} - Defender {})",
                    breakdown.total_score,
                    breakdown.war_goal_score,
                    breakdown.attacker_occupation_score,
                    atk_occ_def,
                    def_owned.len(),
                    breakdown.defender_occupation_score,
                    def_occ_atk,
                    atk_owned.len(),
                    breakdown.battle_score,
                    war.won_attacker_battles,
                    war.won_defender_battles,
                )),
                TextColor(Color::srgb(0.9, 0.9, 0.6)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            // 降伏進捗状況
            let def_cap = check_defender_capitulation(war, &state_registry, &military_registry);
            let atk_cap = check_attacker_capitulation(war, &state_registry, &military_registry);
            let def_has_army = has_combat_ready_armies(def_id, &military_registry);
            let atk_has_army = has_combat_ready_armies(atk_id, &military_registry);

            parent.spawn((
                Text::new(format!(
                    "Capitulation Status:\n Defender ({}) Readiness: Army: {} | Occupied: {}/{} | Cap: {}\n Attacker ({}) Readiness: Army: {} | Occupied: {}/{} | Cap: {}",
                    def_name,
                    if def_has_army { "Ready" } else { "None" },
                    atk_occ_def,
                    def_owned.len(),
                    if def_cap { "YES" } else { "No" },
                    atk_name,
                    if atk_has_army { "Ready" } else { "None" },
                    def_occ_atk,
                    atk_owned.len(),
                    if atk_cap { "YES" } else { "No" },
                )),
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
            ));

            // 講和アクションボタン群
            parent.spawn((
                Text::new("--- Peace Negotiations ---"),
                TextColor(Color::srgb(0.8, 0.8, 0.9)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            let recipient_id = if is_player_attacker { def_id } else { atk_id };

            // 1. 白紙講和ボタン
            let offer_wp = PeaceOffer {
                war_id: war.id,
                proposer_country_id: player_id,
                recipient_country_id: recipient_id,
                term: PeaceTerm::WhitePeace,
                created_date: current_date_str.clone(),
            };
            let wp_check = can_accept_peace_offer(
                &offer_wp,
                &war_registry,
                Some(&state_registry),
                Some(&military_registry),
                &current_date_str,
            );

            parent
                .spawn((
                    PeaceActionButton {
                        war_id: war.id,
                        term: PeaceTerm::WhitePeace,
                    },
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(6.0)),
                        margin: UiRect::top(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(if wp_check.is_ok() {
                        Color::srgba(0.2, 0.4, 0.6, 0.9)
                    } else {
                        Color::srgba(0.2, 0.2, 0.2, 0.6)
                    }),
                ))
                .with_children(|btn| {
                    let label = match wp_check {
                        Ok(()) => "Propose White Peace (Available)".to_string(),
                        Err(msg) => format!("Propose White Peace [Unavailable: {}]", msg),
                    };
                    btn.spawn((
                        Text::new(label),
                        TextColor(Color::WHITE),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                    ));
                });

            // 2. 戦争目標地域の割譲要求ボタン (攻撃側のみ)
            if is_player_attacker {
                let offer_cede = PeaceOffer {
                    war_id: war.id,
                    proposer_country_id: player_id,
                    recipient_country_id: recipient_id,
                    term: PeaceTerm::CedeWarGoalRegion,
                    created_date: current_date_str.clone(),
                };
                let cede_check = can_accept_peace_offer(
                    &offer_cede,
                    &war_registry,
                    Some(&state_registry),
                    Some(&military_registry),
                    &current_date_str,
                );

                parent
                    .spawn((
                        PeaceActionButton {
                            war_id: war.id,
                            term: PeaceTerm::CedeWarGoalRegion,
                        },
                        Button,
                        Node {
                            padding: UiRect::all(Val::Px(6.0)),
                            margin: UiRect::top(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(if cede_check.is_ok() {
                            Color::srgba(0.6, 0.3, 0.2, 0.9)
                        } else {
                            Color::srgba(0.2, 0.2, 0.2, 0.6)
                        }),
                    ))
                    .with_children(|btn| {
                        let label = match cede_check {
                            Ok(()) => "Demand Cession of War Goal Region (Available)".to_string(),
                            Err(msg) => format!("Demand Cession of War Goal Region [{}]", msg),
                        };
                        btn.spawn((
                            Text::new(label),
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
            }

            // 3. 攻撃側の敗北受諾ボタン (攻撃側のみ)
            if is_player_attacker {
                parent
                    .spawn((
                        PeaceActionButton {
                            war_id: war.id,
                            term: PeaceTerm::AttackerConcedes,
                        },
                        Button,
                        Node {
                            padding: UiRect::all(Val::Px(6.0)),
                            margin: UiRect::top(Val::Px(2.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.5, 0.2, 0.2, 0.9)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("Concede Defeat (Surrender)"),
                            TextColor(Color::WHITE),
                            TextFont {
                                font_size: FontSize::Px(11.0),
                                ..default()
                            },
                        ));
                    });
            }
        } else {
            parent.spawn((
                Text::new("No active war engaging your country."),
                TextColor(Color::srgb(0.5, 0.8, 0.5)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));
        }

        // 休戦情報
        let mut truces = Vec::new();
        for country in country_registry.countries.iter() {
            if country.id == player_id {
                continue;
            }
            if let Some(t_until) = diplomacy_registry.get_truce_until(player_id, country.id, &current_date_str) {
                truces.push((country.name.clone(), t_until));
            }
        }

        if !truces.is_empty() {
            parent.spawn((
                Text::new("--- Active Truces ---"),
                TextColor(Color::srgb(0.7, 0.8, 0.9)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
            for (c_name, t_until) in truces {
                parent.spawn((
                    Text::new(format!(" Truce with {}: Until {}", c_name, t_until)),
                    TextColor(Color::srgb(0.8, 0.8, 0.6)),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                ));
            }
        }

        // 過去の戦争履歴 (History)
        let ended_wars: Vec<_> = war_registry
            .wars
            .values()
            .filter(|w| {
                w.status != WarStatus::Active
                    && (w.attackers.contains(&player_id) || w.defenders.contains(&player_id))
            })
            .collect();

        if !ended_wars.is_empty() {
            parent.spawn((
                Text::new("--- Past War History ---"),
                TextColor(Color::srgb(0.7, 0.7, 0.8)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            for war in ended_wars {
                let outcome_str = match war.status {
                    WarStatus::AttackerVictory => "Attacker Victory",
                    WarStatus::DefenderVictory => "Defender Victory",
                    WarStatus::WhitePeace => "White Peace",
                    WarStatus::Cancelled => "Cancelled",
                    _ => "Ended",
                };
                let terms = war.applied_terms.join(", ");

                parent.spawn((
                    Text::new(format!(
                        "• {} | Outcome: {} | Terms: {} | Ended: {} | Duration: {} days | Final Score: {:.0}",
                        war.name,
                        outcome_str,
                        if terms.is_empty() { "None" } else { &terms },
                        war.end_date.as_deref().unwrap_or("N/A"),
                        war.duration_days,
                        war.war_score
                    )),
                    TextColor(Color::srgb(0.7, 0.7, 0.7)),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                ));
            }
        }
    });
}
