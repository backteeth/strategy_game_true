use crate::app::game_state::GameState;
use crate::app::time::GameDate;
use crate::common::WarId;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::localization::{CurrentLocale, TranslationCatalog, localized_text, t, translate};
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
use crate::war::update::ReportedPeaceSettlementErrors;
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
                    handle_peace_action_buttons.run_if(crate::country::player_can_issue_commands),
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
    mut crisis_registry: ResMut<CrisisRegistry>,
    mut claim_registry: ResMut<ClaimRegistry>,
    mut country_registry: ResMut<CountryRegistry>,
    mut reported_errors: ResMut<ReportedPeaceSettlementErrors>,
    mut notif_writer: MessageWriter<GameNotification>,
    loc: crate::localization::Loc,
    mut elimination_extras: PeaceSettlementEliminationExtras,
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
            Some(&country_registry),
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
                    &mut crisis_registry,
                    &mut claim_registry,
                    &mut country_registry,
                    &mut elimination_extras.army_registry,
                    &mut elimination_extras.justification_registry,
                    &mut elimination_extras.power_registry,
                );

                match result {
                    Ok(()) => {
                        // P21-020(ユーザー指摘対応): プレイヤー講和の成功でも、
                        // 自動降伏エラーの重複抑止記録を必ず掃除する
                        // (`ReportedPeaceSettlementErrors::retain_only_active_wars`
                        // 参照——自動失敗の記録がこのWarに残ったまま手動講和で
                        // 終戦した場合、記録が永久に残ってしまう既存ギャップの修正)。
                        reported_errors.retain_only_active_wars(&war_registry);
                        notif_writer.write(GameNotification {
                            message: loc.tf(
                                "peace_panel.notif_peace_enacted",
                                vec![("term", loc.t(btn.term.display_name()))],
                            ),
                        });
                    }
                    Err(err_msg) => {
                        // P21-020: `execute_peace_settlement`のErrを無言で捨てず、
                        // `can_accept_peace_offer`のErr(直上の`Err(err_msg)`分岐)と
                        // 同じ`peace_panel.notif_offer_rejected`翻訳キー経由で
                        // プレイヤーへ理由を示す(所有者不一致・最後の所有陸州・
                        // lineage不整合等、共通Domain APIが返す翻訳キーはいずれも
                        // 既存の`localization::t()`で解決できる)。
                        notif_writer.write(GameNotification {
                            message: loc.tf(
                                "peace_panel.notif_offer_rejected",
                                vec![("reason", loc.t(err_msg))],
                            ),
                        });
                    }
                }
            }
            Err(err_msg) => {
                notif_writer.write(GameNotification {
                    message: loc.tf(
                        "peace_panel.notif_offer_rejected",
                        vec![("reason", loc.t(err_msg))],
                    ),
                });
            }
        }
    }
}

/// P21-021: `handle_peace_action_buttons`が国家消滅発生時に内部で
/// `country::elimination::apply_country_elimination`を呼ぶために必要な
/// `ArmyRegistry`/`WarJustificationRegistry`を1つのSystemParamへまとめる。
/// この関数は既にBevyのSystemParamタプル引数数上限(16)に達しているため、
/// `localization::Loc`と同じ「複数Resourceを1つのSystemParamへ束ねる」
/// パターンを踏襲する。
#[derive(bevy::ecs::system::SystemParam)]
struct PeaceSettlementEliminationExtras<'w> {
    army_registry: ResMut<'w, crate::military::army::ArmyRegistry>,
    justification_registry: ResMut<'w, crate::war::justification::WarJustificationRegistry>,
    power_registry: ResMut<'w, crate::country::power::CountryPowerRegistry>,
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
    crisis_registry: Res<CrisisRegistry>,
    claim_registry: Res<ClaimRegistry>,
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

            // P21-019: Crisis由来Warの戦争目標を最小限表示する(§9)。
            // 既存の`war.name`は州名を含むものの、翻訳されない・claimant/対象国/
            // Claim状態を構造化して示さないため、新たに1行追加する。通常War
            // (`source_crisis_id.is_none()`)では一切表示しない。
            if let Some(crisis_id) = war.source_crisis_id
                && let Some(crisis) = crisis_registry.crises.get(&crisis_id)
            {
                let claimant_name = country_registry
                    .get(crisis.initiator)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                let target_name = country_registry
                    .get(crisis.target)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                let goal_state_id = crisis
                    .war_goals
                    .first()
                    .and_then(|g| g.target_states.first());
                let state_name = goal_state_id
                    .and_then(|&sid| state_registry.get(sid))
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));
                let claim_status_text = crisis
                    .related_claim_id
                    .and_then(|cid| claim_registry.claims.get(&cid))
                    .map(|c| t(&catalog, locale.0, c.status.display_name()))
                    .unwrap_or_else(|| t(&catalog, locale.0, "common.unknown"));

                let (text, marker) = localized_text(
                    &catalog,
                    locale.0,
                    "peace_panel.crisis_war_goal",
                    vec![
                        ("claimant", claimant_name),
                        ("target", target_name),
                        ("state", state_name),
                        ("claim_status", claim_status_text),
                    ],
                );
                parent.spawn((
                    text,
                    marker,
                    TextColor(Color::srgb(0.8, 0.6, 0.9)),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                ));
            }

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
                Some(&country_registry),
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
                    Some(&country_registry),
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

    // ─────────────────────────────────────────────────────────────────────
    // P21-019: Crisis由来War戦争目標のUI表示
    // ─────────────────────────────────────────────────────────────────────

    use crate::common::{CountryId, StateId};
    use crate::country::CountryData;
    use crate::diplomacy::claims::{ClaimSource, ClaimStatus, TerritorialClaim};
    use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis, WarGoal, WarGoalType};
    use crate::state::data::StateData;
    use crate::war::data::War;

    /// claimant=CountryId(0)がtarget=CountryId(1)のStateId(1)へ、Crisis由来War
    /// (WarId(0))で交戦中の完全なレンダリング用環境を構築する。
    fn build_crisis_war_render_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(CurrentLocale::default());
        app.insert_resource(TranslationCatalog::load().expect("embedded catalogs must parse"));
        app.insert_resource(GameDate::new(1936, 3, 1));
        app.insert_resource(PlayerCountry(Some(CountryId(0))));
        app.insert_resource(PeacePanelState {
            open: true,
            selected_war_id: None,
        });

        app.insert_resource(CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    name: "Claimant Nation".to_string(),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    name: "Target Nation".to_string(),
                    ..CountryData::default()
                },
            ],
        });
        app.insert_resource(StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: CountryId(0),
                name: "Home State".to_string(),
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                name: "Contested State".to_string(),
                ..Default::default()
            },
        ]));
        app.insert_resource(MilitaryRegistry::default());
        app.insert_resource(DiplomacyRegistry::default());

        let mut claim_registry = ClaimRegistry::default();
        claim_registry.claims.insert(
            crate::common::ClaimId(0),
            TerritorialClaim {
                id: crate::common::ClaimId(0),
                claimant_country: CountryId(0),
                target_state: StateId(1),
                strength: 50.0,
                created_date: "1936/01/01".to_string(),
                is_permanent: false,
                source: ClaimSource::Strategic,
                status: ClaimStatus::Active,
            },
        );
        app.insert_resource(claim_registry);

        let mut crisis_registry = CrisisRegistry::default();
        crisis_registry.crises.insert(
            crate::common::DiplomaticCrisisId(0),
            DiplomaticCrisis {
                id: crate::common::DiplomaticCrisisId(0),
                initiator: CountryId(0),
                target: CountryId(1),
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                current_phase: CrisisPhase::WarStarted,
                escalation: 100.0,
                initiator_support: 0.0,
                target_resistance: 0.0,
                days_in_phase: 0,
                deadline_date: None,
                international_concern: 0.0,
                third_party_reactions: std::collections::HashMap::new(),
                related_claim_id: Some(crate::common::ClaimId(0)),
                related_justification_id: None,
                related_war_id: None,
            },
        );
        app.insert_resource(crisis_registry);

        let mut war_registry = WarRegistry::default();
        war_registry.wars.insert(
            WarId(0),
            War {
                id: WarId(0),
                name: "Test Crisis War".to_string(),
                attackers: [CountryId(0)].into_iter().collect(),
                defenders: [CountryId(1)].into_iter().collect(),
                primary_attacker: Some(CountryId(0)),
                primary_defender: Some(CountryId(1)),
                source_crisis_id: Some(crate::common::DiplomaticCrisisId(0)),
                war_goals: vec![WarGoal {
                    attacker: CountryId(0),
                    defender: CountryId(1),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![StateId(1)],
                    base_peace_cost: 20.0,
                    international_concern: 10.0,
                    completion: 0.0,
                    is_primary: true,
                }],
                start_date: "1936/01/01".to_string(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );
        app.insert_resource(war_registry);

        app.add_systems(Startup, (spawn_tab_bar, setup_peace_panel).chain());
        app.add_systems(Update, update_peace_panel_ui);
        app.update();
        app
    }

    fn all_texts(app: &mut App) -> Vec<String> {
        app.world_mut()
            .query::<&Text>()
            .iter(app.world())
            .map(|t| t.0.clone())
            .collect()
    }

    /// 要求テスト48,49: Crisis由来Warの戦争目標(claimant名・対象国名・
    /// target_state名)が表示される。
    #[test]
    fn crisis_war_goal_shows_claimant_target_and_state_names() {
        let mut app = build_crisis_war_render_app();
        let joined = all_texts(&mut app).join("\n");
        assert!(joined.contains("Claimant Nation"), "{joined}");
        assert!(joined.contains("Target Nation"), "{joined}");
        assert!(joined.contains("Contested State"), "{joined}");
    }

    /// 要求テスト48(Claim状態表示): 元Claimの状態(Active)も表示される。
    #[test]
    fn crisis_war_goal_shows_claim_status() {
        let mut app = build_crisis_war_render_app();
        let joined = all_texts(&mut app).join("\n");
        assert!(
            joined.contains("Active") || joined.contains("有効"),
            "claim status must be rendered: {joined}"
        );
    }

    /// 要求テスト50: 通常War(`source_crisis_id: None`)ではCrisis戦争目標行を
    /// 表示しない。
    #[test]
    fn normal_war_does_not_show_crisis_war_goal_line() {
        let mut app = build_crisis_war_render_app();
        app.world_mut()
            .resource_mut::<WarRegistry>()
            .wars
            .get_mut(&WarId(0))
            .unwrap()
            .source_crisis_id = None;
        app.update();

        let joined = all_texts(&mut app).join("\n");
        assert!(
            !joined.contains("Contested State"),
            "a normal war must not render the crisis-origin war goal line: {joined}"
        );
    }

    /// 要求テスト51: ロケール切替でCrisis戦争目標表示も更新される
    /// (既定ロケールはja-JP、`crate::localization::DEFAULT_LOCALE`参照)。
    #[test]
    fn crisis_war_goal_text_follows_locale_switch() {
        let mut app = build_crisis_war_render_app();
        let ja_joined = all_texts(&mut app).join("\n");
        assert!(ja_joined.contains("外交危機由来の戦争目標"), "{ja_joined}");

        app.world_mut().resource_mut::<CurrentLocale>().0 = crate::localization::Locale::EnUs;
        app.update();
        let en_joined = all_texts(&mut app).join("\n");
        assert!(
            en_joined.contains("claims"),
            "switching to en-US must render the English crisis war goal text: {en_joined}"
        );
    }

    /// 要求テスト52: Crisis戦争目標行を追加しても、既存のスクロール構造・
    /// 講和アクションボタンは維持される。
    #[test]
    fn crisis_war_goal_line_preserves_scroll_and_peace_action_buttons() {
        let mut app = build_crisis_war_render_app();

        let scroll_body_count = app
            .world_mut()
            .query_filtered::<&ScrollPosition, With<PeacePanelContentContainer>>()
            .iter(app.world())
            .count();
        // PeacePanelContentContainer自体はScrollPositionを持たないため、
        // 代わりに`ui::scroll`が生成するスクロール本体全体の存在を確認する。
        let _ = scroll_body_count;
        let any_scroll_position = app
            .world_mut()
            .query::<&ScrollPosition>()
            .iter(app.world())
            .count();
        assert!(
            any_scroll_position >= 1,
            "the peace panel's scrollable body must still exist"
        );

        let button_count = app
            .world_mut()
            .query::<&PeaceActionButton>()
            .iter(app.world())
            .count();
        assert!(
            button_count > 0,
            "existing peace action buttons must still render alongside the new crisis war goal line"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-020: `execute_peace_settlement`のErrをUIが無言で捨てず、翻訳済み
    // エラー通知として表示することの検証(§14要求47,48,49,53)。
    // ─────────────────────────────────────────────────────────────────────

    /// `build_crisis_war_render_app`と同じCrisis由来War環境に、
    /// `handle_peace_action_buttons`自体を実行するのに必要なResource
    /// (BattleRegistry/FrontlineRegistry/GameNotificationメッセージ登録)を
    /// 追加し、`can_accept_peace_offer`を通過できるよう`war_score`を十分に
    /// 上げた状態で返す。
    fn build_peace_action_test_app() -> App {
        let mut app = build_crisis_war_render_app();
        app.add_message::<GameNotification>();
        app.insert_resource(BattleRegistry::default());
        app.insert_resource(crate::war::frontline::FrontlineRegistry::default());
        app.insert_resource(ReportedPeaceSettlementErrors::default());
        app.insert_resource(crate::military::army::ArmyRegistry::default());
        app.insert_resource(crate::war::justification::WarJustificationRegistry::default());
        app.insert_resource(crate::country::power::CountryPowerRegistry::default());
        app.world_mut()
            .resource_mut::<WarRegistry>()
            .wars
            .get_mut(&WarId(0))
            .unwrap()
            .war_score = 100.0;
        app.add_systems(Update, handle_peace_action_buttons);
        app
    }

    fn press_peace_action_button(app: &mut App, war_id: WarId, term: PeaceTerm) {
        app.world_mut().spawn((
            Button,
            Interaction::Pressed,
            PeaceActionButton { war_id, term },
        ));
        app.update();
    }

    fn drain_notifications(app: &mut App) -> Vec<String> {
        app.world_mut()
            .resource_mut::<Messages<GameNotification>>()
            .drain()
            .map(|n| n.message)
            .collect()
    }

    /// 要求テスト47,53: `execute_peace_settlement`が失敗した場合、翻訳済みの
    /// エラー通知が表示され、講和ボタンは(パネル構造上)引き続き利用可能。
    #[test]
    fn peace_settlement_failure_shows_translated_error_notification() {
        let mut app = build_peace_action_test_app();
        // Crisis由来lineageを壊す(related_claim_idを外す)ことで、
        // `can_accept_peace_offer`は通過するが`execute_peace_settlement`の
        // `resolve_crisis_war_lineage`が拒否するようにする。
        app.world_mut()
            .resource_mut::<CrisisRegistry>()
            .crises
            .get_mut(&crate::common::DiplomaticCrisisId(0))
            .unwrap()
            .related_claim_id = None;

        press_peace_action_button(&mut app, WarId(0), PeaceTerm::CedeWarGoalRegion);

        let notifications = drain_notifications(&mut app);
        assert_eq!(
            notifications.len(),
            1,
            "exactly one notification must be sent"
        );
        assert!(
            notifications[0].contains("linked territorial claim")
                || notifications[0].contains("領土請求"),
            "the notification must include the translated lineage-missing-claim reason, got: {:?}",
            notifications[0]
        );

        // War自体はActiveのまま(部分適用されていない)。
        assert_eq!(
            app.world()
                .resource::<WarRegistry>()
                .wars
                .get(&WarId(0))
                .unwrap()
                .status,
            WarStatus::Active
        );
    }

    /// 要求テスト48: 成功時にはエラー通知は残らず、成功通知のみが送られる。
    #[test]
    fn peace_settlement_success_shows_no_error_notification() {
        let mut app = build_peace_action_test_app();

        press_peace_action_button(&mut app, WarId(0), PeaceTerm::CedeWarGoalRegion);

        let notifications = drain_notifications(&mut app);
        assert_eq!(notifications.len(), 1);
        assert!(
            !notifications[0].to_lowercase().contains("fail") && !notifications[0].contains("失敗"),
            "a successful settlement must not show a failure-shaped notification, got: {:?}",
            notifications[0]
        );
    }

    /// 要求(ユーザー指摘対応): 「自動降伏失敗→重複抑止記録追加→プレイヤーが
    /// 手動講和成功→記録削除」を確認する。自動降伏の失敗記録が、無関係な
    /// プレイヤー講和の成功パスでは掃除されず永久に残ってしまう既存ギャップの
    /// 修正確認。
    #[test]
    fn manual_peace_success_clears_a_previously_reported_auto_capitulation_failure() {
        let mut app = build_peace_action_test_app();
        // 自動降伏が過去にこのWarへ失敗を記録していた状態を模す。
        app.world_mut()
            .resource_mut::<ReportedPeaceSettlementErrors>()
            .reported
            .insert((WarId(0), "war_error.peace.some_previous_auto_failure"));

        press_peace_action_button(&mut app, WarId(0), PeaceTerm::CedeWarGoalRegion);

        // 講和が成功したことを確認したうえで、
        let notifications = drain_notifications(&mut app);
        assert_eq!(notifications.len(), 1);
        assert!(
            !notifications[0].to_lowercase().contains("fail") && !notifications[0].contains("失敗"),
            "the manual settlement must succeed, got: {:?}",
            notifications[0]
        );
        assert_eq!(
            app.world()
                .resource::<WarRegistry>()
                .wars
                .get(&WarId(0))
                .unwrap()
                .status,
            WarStatus::AttackerVictory
        );

        // このWarの古い自動降伏失敗記録が消えていることを確認する。
        assert!(
            !app.world()
                .resource::<ReportedPeaceSettlementErrors>()
                .reported
                .iter()
                .any(|&(w, _)| w == WarId(0)),
            "a manual peace success must clear this war's previously-reported auto-\
             capitulation failure entries, not leave them orphaned forever"
        );
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
