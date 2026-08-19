use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::WarId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text, t, tf, translate};
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::ui::tab_bar::{TabBarRoot, ctrl_held};
use crate::war::capitulation::{
    check_attacker_capitulation, check_defender_capitulation, has_combat_ready_divisions,
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
            .add_systems(
                OnEnter(GameState::Playing),
                setup_peace_panel.after(crate::ui::tab_bar::spawn_tab_bar),
            )
            .add_systems(
                Update,
                (
                    toggle_peace_panel,
                    handle_peace_action_buttons,
                    update_peace_panel_ui,
                )
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_peace_panel(
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
                    TogglePeacePanelButton,
                    Button,
                    Node {
                        padding: UiRect::all(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.3, 0.2, 0.4, 0.9)),
                ))
                .with_children(|btn| {
                    let (text, marker) =
                        localized_text(&catalog, locale.0, "peace_panel.toggle_button", vec![]);
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

    // Peace Panel Window
    // バグ報告(2026-08-17): 講和パネルだけ他4パネル(left:310,top:75,幅580〜600)と
    // 異なる原点・幅(left:250,width:700)で開かれ、視覚的に「他より大きい」ように
    // 見えていた。他4パネルと同じ原点・同程度の幅へ統一する。
    // タイトルはスクロール対象の外(常に表示)、それ以外はスクロール可能な本体
    // (`ui::scroll::spawn_scrollable_body`)へ入れる(詳細は`ui::scroll`のドキュメント参照)。
    let peace_panel_entity = commands
        .spawn((
            PeacePanelRoot,
            // P21-013: 背景自体もButton化し、子Button以外の余白をクリック/ホバーしても
            // `Interaction`を確実に発行させる(`ui::load_confirm`の既存パターンを踏襲)。
            Button,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(310.0),
                top: Val::Px(75.0),
                width: Val::Px(600.0),
                height: Val::Px(600.0),
                padding: UiRect::all(Val::Px(16.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.08, 0.12, 0.95)),
        ))
        .id();
    commands.entity(peace_panel_entity).with_children(|parent| {
        let (text, marker) = localized_text(&catalog, locale.0, "peace_panel.title", vec![]);
        parent.spawn((
            text,
            marker,
            TextColor(Color::srgb(0.9, 0.8, 0.4)),
            TextFont {
                font_size: FontSize::Px(18.0),
                ..default()
            },
        ));

        crate::ui::scroll::spawn_scrollable_body(parent, Val::Px(8.0), |parent| {
            parent.spawn((
                PeacePanelContentContainer,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    flex_grow: 1.0,
                    ..default()
                },
            ));
        });
    });
}

/// 他4パネル(`research_panel::toggle_research_panel_key`等)と同じ構造:
/// `ActivePanel`を経由することで、他タブへ切り替えたときに講和パネルも自動的に閉じる
/// (`ui::mod::sync_panels_to_active`が`ActivePanel`の変化を見て`PeacePanelState.open`を
/// 同期する)。講和パネル自身のopen/closeは、ここで即座に`state.open`へも反映する
/// (他4パネルが`panel_q.single_mut().display`を直接書き換えるのと同じく、
/// 自パネル分の反映をシステム間の実行順に依存させないため)。
fn toggle_peace_panel(
    mut state: ResMut<PeacePanelState>,
    mut active_panel: ResMut<crate::ui::ActivePanel>,
    keys: Res<ButtonInput<KeyCode>>,
    btn_q: Query<&Interaction, (With<TogglePeacePanelButton>, Changed<Interaction>)>,
) {
    let mut toggle = false;
    // Ctrl+4: タブ切替は全パネル共通でCtrl+数字に統一
    // (軍事パネル内の素のDigit1/2/3/7/8/9フロントライン操作キーとの衝突を避けるため)。
    if keys.just_pressed(KeyCode::Digit4) && ctrl_held(&keys) {
        toggle = true;
    }
    for interaction in btn_q.iter() {
        if *interaction == Interaction::Pressed {
            toggle = true;
        }
    }

    if toggle {
        active_panel.toggle(crate::ui::PanelKind::Peace);
        state.open = active_panel.current == crate::ui::PanelKind::Peace;
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
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
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

        // P21-016: 講和申し入れの相手は常に敵陣営の代表国(primary)とする
        // (支持国宛てに送る講和申し入れは既存仕様の対象外)。
        let recipient_id = if war.attackers.contains(&player_id) {
            war.primary_defender_id()
        } else {
            war.primary_attacker_id()
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
                // war.end_reasonはゲーム状態フィールドのため、翻訳済み文字列ではなく
                // 安定した翻訳キーそのものを格納する(表示時にpeace_panel側でtranslateする)。
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
                        message: tf(
                            &catalog,
                            locale.0,
                            "peace_panel.notif_peace_enacted",
                            vec![("term", t(&catalog, locale.0, btn.term.display_name()))],
                        ),
                    });
                }
            }
            Err(err_msg) => {
                notif_writer.write(GameNotification {
                    message: tf(
                        &catalog,
                        locale.0,
                        "peace_panel.notif_offer_rejected",
                        vec![("reason", t(&catalog, locale.0, err_msg))],
                    ),
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
    locale: Res<CurrentLocale>,
    catalog: Res<TranslationCatalog>,
    mut q_panel: Query<&mut Node, With<PeacePanelRoot>>,
    q_container: Query<(Entity, Option<&Children>), With<PeacePanelContentContainer>>,
    mut commands: Commands,
) {
    let Ok(mut root_node) = q_panel.single_mut() else {
        return;
    };

    if !panel_state.open && !locale.is_changed() {
        if !panel_state.open {
            root_node.display = Display::None;
        }
        return;
    }

    root_node.display = if panel_state.open {
        Display::Flex
    } else {
        Display::None
    };

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
            let (text, marker) =
                localized_text(&catalog, locale.0, "peace_panel.no_country", vec![]);
            commands.entity(container_entity).with_children(|parent| {
                parent.spawn((text, marker, TextColor(Color::srgb(0.5, 0.5, 0.5))));
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
            let atk_id = war.primary_attacker_id();
            let def_id = war.primary_defender_id();

            let atk_name = country_registry
                .get(atk_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "peace_panel.role_attacker"));
            let def_name = country_registry
                .get(def_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| t(&catalog, locale.0, "peace_panel.role_defender"));

            let is_player_attacker = war.attackers.contains(&player_id);
            let role_key = if is_player_attacker {
                "peace_panel.role_attacker"
            } else {
                "peace_panel.role_defender"
            };

            let start_date = GameDate::from_string(&war.start_date).unwrap_or_default();
            let elapsed_days = game_date.days_since(&start_date).max(0);

            let breakdown = calculate_war_score(war, &state_registry);

            let atk_owned = state_registry.get_owned_states(atk_id);
            let def_owned = state_registry.get_owned_states(def_id);

            let atk_occ_def = def_owned
                .iter()
                .filter(|s| s.controller() == atk_id)
                .count();
            let def_occ_atk = atk_owned
                .iter()
                .filter(|s| s.controller() == def_id)
                .count();

            let (text, marker) = localized_text(
                &catalog,
                locale.0,
                "peace_panel.active_war",
                vec![
                    ("name", war.name.clone()),
                    (
                        "enemy",
                        if is_player_attacker {
                            def_name.clone()
                        } else {
                            atk_name.clone()
                        },
                    ),
                    ("role", t(&catalog, locale.0, role_key)),
                    ("days", elapsed_days.to_string()),
                    ("start", war.start_date.clone()),
                ],
            );
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.4, 0.9, 1.0)),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
            ));

            // 戦勝点内訳
            let (text, marker) = localized_text(
                &catalog,
                locale.0,
                "peace_panel.war_score",
                vec![
                    ("total", breakdown.total_score.to_string()),
                    ("goal", breakdown.war_goal_score.to_string()),
                    ("atk_occ", breakdown.attacker_occupation_score.to_string()),
                    ("atk_occ_count", atk_occ_def.to_string()),
                    ("atk_total", def_owned.len().to_string()),
                    ("def_occ", breakdown.defender_occupation_score.to_string()),
                    ("def_occ_count", def_occ_atk.to_string()),
                    ("def_total", atk_owned.len().to_string()),
                    ("battle", breakdown.battle_score.to_string()),
                    ("atk_wins", war.won_attacker_battles.to_string()),
                    ("def_wins", war.won_defender_battles.to_string()),
                ],
            );
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.9, 0.9, 0.6)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));

            // 降伏進捗状況
            let def_cap = check_defender_capitulation(war, &state_registry, &military_registry);
            let atk_cap = check_attacker_capitulation(war, &state_registry, &military_registry);
            let def_has_division = has_combat_ready_divisions(def_id, &military_registry);
            let atk_has_division = has_combat_ready_divisions(atk_id, &military_registry);

            let ready_key = |ready: bool| {
                if ready {
                    "peace_panel.division_ready"
                } else {
                    "peace_panel.division_none"
                }
            };
            let cap_key = |cap: bool| {
                if cap {
                    "peace_panel.cap_yes"
                } else {
                    "peace_panel.cap_no"
                }
            };

            let (text, marker) = localized_text(
                &catalog,
                locale.0,
                "peace_panel.capitulation_status",
                vec![
                    ("defender", def_name.clone()),
                    (
                        "def_division",
                        t(&catalog, locale.0, ready_key(def_has_division)),
                    ),
                    ("atk_occ", atk_occ_def.to_string()),
                    ("def_total", def_owned.len().to_string()),
                    ("def_cap", t(&catalog, locale.0, cap_key(def_cap))),
                    ("attacker", atk_name.clone()),
                    (
                        "atk_division",
                        t(&catalog, locale.0, ready_key(atk_has_division)),
                    ),
                    ("def_occ", def_occ_atk.to_string()),
                    ("atk_total", atk_owned.len().to_string()),
                    ("atk_cap", t(&catalog, locale.0, cap_key(atk_cap))),
                ],
            );
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
            ));

            // 講和アクションボタン群
            let (text, marker) = localized_text(
                &catalog,
                locale.0,
                "peace_panel.negotiations_header",
                vec![],
            );
            parent.spawn((
                text,
                marker,
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
                    let (text, marker) = match wp_check {
                        Ok(()) => localized_text(
                            &catalog,
                            locale.0,
                            "peace_panel.white_peace_available",
                            vec![],
                        ),
                        Err(msg) => localized_text(
                            &catalog,
                            locale.0,
                            "peace_panel.white_peace_unavailable",
                            vec![("reason", t(&catalog, locale.0, msg))],
                        ),
                    };
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
                        let (text, marker) = match cede_check {
                            Ok(()) => localized_text(
                                &catalog,
                                locale.0,
                                "peace_panel.cede_available",
                                vec![],
                            ),
                            Err(msg) => localized_text(
                                &catalog,
                                locale.0,
                                "peace_panel.cede_unavailable",
                                vec![("reason", t(&catalog, locale.0, msg))],
                            ),
                        };
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
                        let (text, marker) = localized_text(
                            &catalog,
                            locale.0,
                            "peace_panel.concede_button",
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
        } else {
            let (text, marker) =
                localized_text(&catalog, locale.0, "peace_panel.no_active_war", vec![]);
            parent.spawn((
                text,
                marker,
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
            if let Some(t_until) =
                diplomacy_registry.get_truce_until(player_id, country.id, &current_date_str)
            {
                truces.push((country.name.clone(), t_until));
            }
        }

        if !truces.is_empty() {
            let (text, marker) =
                localized_text(&catalog, locale.0, "peace_panel.truces_header", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.7, 0.8, 0.9)),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
            ));
            for (c_name, t_until) in truces {
                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "peace_panel.truce_line",
                    vec![("country", c_name), ("until", t_until)],
                );
                parent.spawn((
                    text,
                    marker,
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
            let (text, marker) =
                localized_text(&catalog, locale.0, "peace_panel.history_header", vec![]);
            parent.spawn((
                text,
                marker,
                TextColor(Color::srgb(0.7, 0.7, 0.8)),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
            ));

            for war in ended_wars {
                let outcome_key = match war.status {
                    WarStatus::AttackerVictory => "war_status.attacker_victory",
                    WarStatus::DefenderVictory => "war_status.defender_victory",
                    WarStatus::WhitePeace => "war_status.white_peace",
                    WarStatus::Cancelled => "war_status.cancelled",
                    _ => "war_status.ended",
                };
                let terms_none = t(&catalog, locale.0, "peace_panel.history_terms_none");
                // applied_termsにはゲーム状態として安定した翻訳キーが格納されている
                // (翻訳済み文字列そのものではない)。表示直前にここで解決する。
                let terms = war
                    .applied_terms
                    .iter()
                    .map(|key| translate(&catalog, locale.0, key, &[]))
                    .collect::<Vec<_>>()
                    .join(", ");
                let end_date = war
                    .end_date
                    .clone()
                    .unwrap_or_else(|| t(&catalog, locale.0, "peace_panel.history_end_date_na"));

                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "peace_panel.history_line",
                    vec![
                        ("name", war.name.clone()),
                        ("outcome", t(&catalog, locale.0, outcome_key)),
                        ("terms", if terms.is_empty() { terms_none } else { terms }),
                        ("end_date", end_date),
                        ("days", war.duration_days.to_string()),
                        ("score", format!("{:.0}", war.war_score)),
                    ],
                );
                parent.spawn((
                    text,
                    marker,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_bar::spawn_tab_bar;

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PeacePanelState::default());
        app.insert_resource(crate::ui::ActivePanel::default());
        app.init_resource::<ButtonInput<KeyCode>>();
        app.add_systems(Update, toggle_peace_panel);
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
    fn ctrl_plus_digit4_toggles_peace_panel() {
        let mut app = build_test_app();
        press_ctrl_plus(&mut app, KeyCode::Digit4);

        assert!(app.world().resource::<PeacePanelState>().open);
        assert_eq!(
            app.world().resource::<crate::ui::ActivePanel>().current,
            crate::ui::PanelKind::Peace
        );
    }

    #[test]
    fn bare_digit4_alone_does_not_toggle_peace_panel() {
        let mut app = build_test_app();
        press_bare(&mut app, KeyCode::Digit4);

        assert!(
            !app.world().resource::<PeacePanelState>().open,
            "Digit4 without Ctrl must not open the Peace panel"
        );
    }

    /// バグ報告(2026-08-17): 講和タブだけ、他タブへ切り替えても開いたままだった。
    /// 講和パネルは`ActivePanel`を経由するようになったため、`ui::mod::sync_panels_to_active`
    /// が他パネルへの切替時に`PeacePanelState.open`をfalseへ同期することを確認する。
    /// `toggle_peace_panel`は意図的に登録しない(このSystemは`sync_panels_to_active`単体の
    /// 動作だけを検証する)。
    #[test]
    fn switching_to_another_panel_closes_the_peace_panel() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PeacePanelState {
            open: true,
            ..Default::default()
        });
        app.insert_resource(crate::ui::ActivePanel {
            current: crate::ui::PanelKind::Peace,
        });
        app.add_systems(Update, crate::ui::sync_panels_to_active);
        assert!(app.world().resource::<PeacePanelState>().open);

        app.world_mut()
            .resource_mut::<crate::ui::ActivePanel>()
            .toggle(crate::ui::PanelKind::Research);
        app.update();

        assert!(
            !app.world().resource::<PeacePanelState>().open,
            "opening a different panel via ActivePanel must close the Peace panel too"
        );
    }

    #[test]
    fn toggle_button_is_spawned_as_a_child_of_the_shared_tab_bar() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, (spawn_tab_bar, setup_peace_panel).chain());
        app.update();

        let tab_bar = app
            .world_mut()
            .query_filtered::<Entity, With<crate::ui::tab_bar::TabBarRoot>>()
            .single(app.world())
            .expect("TabBarRoot must be spawned");
        let button = app
            .world_mut()
            .query_filtered::<Entity, With<TogglePeacePanelButton>>()
            .single(app.world())
            .expect("TogglePeacePanelButton must be spawned");

        assert_eq!(
            app.world().entity(button).get::<ChildOf>().map(|c| c.0),
            Some(tab_bar),
            "the peace toggle button must be a child of the shared TabBarRoot"
        );
    }

    /// P21-013: `PeacePanelRoot`自身が`Button`であることを確認する回帰テスト。これにより
    /// 子Button以外の余白をクリック/ホバーしても`Interaction`が確実に発行され、
    /// `map::selection::handle_state_click`等の既存「UIのHovered/Pressed中はマップ操作を
    /// スキップする」ガードがこの領域にも効くようになる(`ui::load_confirm`の既存パターンと
    /// 同じ)。
    #[test]
    fn peace_panel_root_background_is_itself_a_button() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.add_systems(Startup, setup_peace_panel);
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<PeacePanelRoot>>()
            .single(app.world())
            .expect("PeacePanelRoot must be spawned");
        assert!(
            app.world().entity(root).contains::<Button>(),
            "PeacePanelRoot's own background must be a Button so hovering it registers Interaction"
        );
    }
}
