use crate::app::game_state::GameState;
use crate::country::PlayerCountry;
use crate::war::data::{WarRegistry, WarStatus};
use bevy::prelude::*;

#[derive(Component)]
pub struct PeacePanelRoot;

pub struct PeacePanelPlugin;

impl Plugin for PeacePanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Playing), setup_peace_panel)
            .add_systems(
                Update,
                update_peace_panel.run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_peace_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(60.0),
                height: Val::Percent(70.0),
                position_type: PositionType::Absolute,
                left: Val::Percent(20.0),
                top: Val::Percent(15.0),
                flex_direction: FlexDirection::Column,
                display: Display::None, // Hidden by default
                padding: UiRect::all(Val::Px(16.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.1, 0.1, 0.1, 0.95)),
            PeacePanelRoot,
        ))
        .with_children(|parent| {
            parent.spawn((Text::new("Peace Conference"),));

            parent.spawn((
                Text::new("A war has concluded. Peace terms are being negotiated..."),
                Node {
                    margin: UiRect::top(Val::Px(10.0)),
                    ..default()
                },
            ));

            // Add more UI here later (e.g. state selection)
        });
}

fn update_peace_panel(
    war_registry: Res<WarRegistry>,
    player_country: Res<PlayerCountry>,
    mut q_panel: Query<&mut Node, With<PeacePanelRoot>>,
) {
    // Show peace panel automatically if player's war is in PeaceNegotiation
    let mut show_panel = false;

    if let Some(country_id) = player_country.0 {
        if let Some(_war) = war_registry.get_active_war_for_country(country_id) {
            // Wait, get_active_war_for_country checks for WarStatus::Active
            // We need to find if there's a peace negotiation
            // ...
            // Let's just do it manually
        }

        for war in war_registry.wars.values() {
            if (war.attackers.contains(&country_id) || war.defenders.contains(&country_id))
                && war.status == WarStatus::PeaceNegotiation
            {
                show_panel = true;
                break;
            }
        }
    }

    for mut node in q_panel.iter_mut() {
        if show_panel {
            node.display = Display::Flex;
        } else {
            node.display = Display::None;
        }
    }
}
