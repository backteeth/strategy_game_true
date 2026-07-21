pub mod construction;
pub mod data;

use bevy::prelude::*;
use data::BuildingRegistry;

pub struct BuildingPlugin;

impl Plugin for BuildingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BuildingRegistry::default());
    }
}
