use crate::app::game_state::GameState;
use crate::common::StateId;
use crate::country::PlayerCountry;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::{FrontlineRegistry, FrontlineStance};
use bevy::prelude::*;
use std::collections::HashSet;

/// 前線オーバーレイ表示のトグルリソース
#[derive(Resource, Debug, Clone)]
pub struct FrontlineRenderSettings {
    pub visible: bool,
}

impl Default for FrontlineRenderSettings {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// 前線ハイライトスプライトのマーカー
#[derive(Component)]
pub struct FrontlineOverlayVisual;

pub struct FrontlineRenderPlugin;

impl Plugin for FrontlineRenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FrontlineRenderSettings::default())
            .add_systems(
                Update,
                (toggle_frontline_overlay_key, update_frontline_overlay)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn toggle_frontline_overlay_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<FrontlineRenderSettings>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        settings.visible = !settings.visible;
    }
}

fn update_frontline_overlay(
    mut commands: Commands,
    settings: Res<FrontlineRenderSettings>,
    player_country: Res<PlayerCountry>,
    war_registry: Res<WarRegistry>,
    state_registry: Res<StateRegistry>,
    frontline_registry: Res<FrontlineRegistry>,
    overlay_q: Query<Entity, With<FrontlineOverlayVisual>>,
) {
    // 既存のオーバーレイをクリア
    for entity in overlay_q.iter() {
        commands.entity(entity).despawn();
    }

    if !settings.visible {
        return;
    }

    let Some(player_cid) = player_country.0 else {
        return;
    };

    // プレイヤーが参加中のアクティブ戦争前線を対象にする
    let active_wars: Vec<_> = war_registry
        .wars
        .values()
        .filter(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&player_cid) || w.defenders.contains(&player_cid))
        })
        .collect();

    for war in active_wars {
        let Some(frontline) = frontline_registry.get_frontline_for_war(war.id) else {
            continue;
        };

        let is_attacker = war.attackers.contains(&player_cid);
        let (my_front, enemy_front) = if is_attacker {
            (
                &frontline.attacker_front_regions,
                &frontline.defender_front_regions,
            )
        } else {
            (
                &frontline.defender_front_regions,
                &frontline.attacker_front_regions,
            )
        };

        let plan = frontline_registry.get_plan(frontline.frontline_id, player_cid);
        let objective = plan.and_then(|p| p.objective_region_id);
        let stance = plan.map(|p| p.stance).unwrap_or(FrontlineStance::Stopped);

        // 自国側前線地域のハイライト (水色/青系)
        let my_color = match stance {
            FrontlineStance::Stopped => Color::srgba(0.2, 0.6, 0.9, 0.45),
            FrontlineStance::Defend => Color::srgba(0.1, 0.8, 0.4, 0.5),
            FrontlineStance::Offensive => Color::srgba(0.9, 0.3, 0.2, 0.5),
        };

        for &state_id in my_front {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                let size = state.rect_size() + Vec2::new(6.0, 6.0);

                commands.spawn((
                    Sprite {
                        color: my_color,
                        custom_size: Some(size),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, 1.2),
                    FrontlineOverlayVisual,
                ));
            }
        }

        // 敵側前線地域のハイライト (赤/橙系)
        let enemy_set: HashSet<StateId> = enemy_front.iter().copied().collect();
        for &state_id in &enemy_set {
            if let Some(state) = state_registry.get(state_id) {
                let pos = state.position();
                let size = state.rect_size() + Vec2::new(4.0, 4.0);

                commands.spawn((
                    Sprite {
                        color: Color::srgba(0.9, 0.1, 0.1, 0.35),
                        custom_size: Some(size),
                        ..default()
                    },
                    Transform::from_xyz(pos.x, pos.y, 1.15),
                    FrontlineOverlayVisual,
                ));
            }
        }

        // 攻勢目標地域のハイライト (金/黄色枠)
        if let Some(obj_id) = objective
            && let Some(state) = state_registry.get(obj_id)
        {
            let pos = state.position();
            let size = state.rect_size() + Vec2::new(10.0, 10.0);

            commands.spawn((
                Sprite {
                    color: Color::srgba(1.0, 0.85, 0.1, 0.6),
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, 1.25),
                FrontlineOverlayVisual,
            ));
        }
    }
}
