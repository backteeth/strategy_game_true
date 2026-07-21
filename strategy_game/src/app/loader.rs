use crate::app::game_state::GameState;
use crate::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use crate::common::StateId;
use crate::country::{CountryData, CountryRegistry};
use crate::economy::resources::StateResourceDeposit;
use crate::state::data::{StateData, StateRegistry};
use bevy::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;

/// ローダープラグイン
pub struct DataLoaderPlugin;

impl Plugin for DataLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            load_game_data.before(crate::app::loader::transition_to_country_selection),
        )
        .add_systems(Startup, transition_to_country_selection);
    }
}

/// RONファイルからゲームデータを読み込む
pub fn load_game_data(
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut building_registry: ResMut<BuildingRegistry>,
) {
    // ── 建物データ読み込み ───────────────────────────────────────────────
    let buildings_ron = std::fs::read_to_string("assets/data/buildings.ron").unwrap_or_else(|e| {
        panic!(
            "[DataLoader] Failed to read assets/data/buildings.ron: {e}\n\
                 Make sure to run the game from the project root directory."
        )
    });

    let building_defs: Vec<BuildingDefinition> = ron::from_str(&buildings_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/buildings.ron: {e}"));

    let mut building_map = HashMap::new();
    for b in building_defs {
        if b.construction_cost < 0.0 {
            panic!(
                "[DataLoader] assets/data/buildings.ron: Building '{:?}' has negative construction cost: {}",
                b.building_type, b.construction_cost
            );
        }
        if b.required_progress <= 0.0 {
            panic!(
                "[DataLoader] assets/data/buildings.ron: Building '{:?}' has invalid required_progress (<= 0): {}",
                b.building_type, b.required_progress
            );
        }
        if b.max_level == 0 {
            panic!(
                "[DataLoader] assets/data/buildings.ron: Building '{:?}' has max_level of 0",
                b.building_type
            );
        }
        for (res, &amount) in &b.input_resources {
            if amount < 0.0 {
                panic!(
                    "[DataLoader] assets/data/buildings.ron: Building '{:?}' has negative input resource amount for {:?}: {}",
                    b.building_type, res, amount
                );
            }
        }
        for (res, &amount) in &b.output_resources {
            if amount < 0.0 {
                panic!(
                    "[DataLoader] assets/data/buildings.ron: Building '{:?}' has negative output resource amount for {:?}: {}",
                    b.building_type, res, amount
                );
            }
        }
        building_map.insert(b.building_type, b);
    }
    building_registry.definitions = building_map;

    // ── 資源鉱床データ読み込み ───────────────────────────────────────────
    let resources_ron = std::fs::read_to_string("assets/data/resources.ron").unwrap_or_else(|e| {
        panic!(
            "[DataLoader] Failed to read assets/data/resources.ron: {e}\n\
                 Make sure to run the game from the project root directory."
        )
    });

    let resource_deposits_map: HashMap<StateId, Vec<StateResourceDeposit>> =
        ron::from_str(&resources_ron).unwrap_or_else(|e| {
            panic!("[DataLoader] Failed to parse assets/data/resources.ron: {e}")
        });

    // ── 国家データ読み込み ───────────────────────────────────────────────
    let countries_ron = std::fs::read_to_string("assets/data/countries.ron").unwrap_or_else(|e| {
        panic!(
            "[DataLoader] Failed to read assets/data/countries.ron: {e}\n\
                 Make sure to run the game from the project root directory."
        )
    });

    let countries: Vec<CountryData> = ron::from_str(&countries_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/countries.ron: {e}"));

    // ── 州データ読み込み ─────────────────────────────────────────────────
    let states_ron = std::fs::read_to_string("assets/data/states.ron").unwrap_or_else(|e| {
        panic!(
            "[DataLoader] Failed to read assets/data/states.ron: {e}\n\
                 Make sure to run the game from the project root directory."
        )
    });

    let mut states: Vec<StateData> = ron::from_str(&states_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/states.ron: {e}"));

    // 鉱床情報を各州へアタッチ
    for state in states.iter_mut() {
        if let Some(deposits) = resource_deposits_map.get(&state.id) {
            state.resource_deposits = deposits.clone();
        }
    }

    // ── バリデーション ───────────────────────────────────────────────────
    validate_data(&countries, &states, &building_registry.definitions);

    info!(
        "[DataLoader] Successfully loaded {} buildings, {} countries, {} states",
        building_registry.definitions.len(),
        countries.len(),
        states.len()
    );

    // ── Resource に注入 ──────────────────────────────────────────────────
    country_registry.countries = countries;
    *state_registry = StateRegistry::build(states);
}

/// データの整合性を検証する
fn validate_data(
    countries: &[CountryData],
    states: &[StateData],
    building_defs: &HashMap<BuildingType, BuildingDefinition>,
) {
    let mut country_ids = HashSet::new();
    for c in countries {
        if !country_ids.insert(c.id.0) {
            panic!("[DataLoader] Duplicate CountryId: {}", c.id.0);
        }
    }

    let mut state_ids = HashSet::new();
    for s in states {
        if !state_ids.insert(s.id.0) {
            panic!("[DataLoader] Duplicate StateId: {}", s.id.0);
        }
    }

    for s in states {
        if !country_ids.contains(&s.owner_country_id.0) {
            panic!(
                "[DataLoader] State '{}' (id={}) references unknown CountryId: {}",
                s.name, s.id.0, s.owner_country_id.0
            );
        }

        // 初期建物レベルのチェック
        for (&b_type, &lvl) in &s.buildings {
            if let Some(def) = building_defs.get(&b_type) {
                if lvl > def.max_level {
                    panic!(
                        "[DataLoader] State '{}' (id={}) initial building level for {:?} ({}) exceeds max level ({})",
                        s.name, s.id.0, b_type, lvl, def.max_level
                    );
                }
            } else {
                panic!(
                    "[DataLoader] State '{}' (id={}) references unknown BuildingType: {:?}",
                    s.name, s.id.0, b_type
                );
            }
        }
    }

    for c in countries {
        if !state_ids.contains(&c.capital_state_id.0) {
            panic!(
                "[DataLoader] Country '{}' (id={}) references unknown capital StateId: {}",
                c.name, c.id.0, c.capital_state_id.0
            );
        }
        let capital_owner = states
            .iter()
            .find(|s| s.id == c.capital_state_id)
            .map(|s| s.owner_country_id);

        if capital_owner != Some(c.id) {
            panic!(
                "[DataLoader] Country '{}' (id={}) capital state {} is not owned by that country (owner: {:?})",
                c.name, c.id.0, c.capital_state_id.0, capital_owner
            );
        }
    }
}

/// データ読み込み完了後に CountrySelection へ遷移する
pub fn transition_to_country_selection(mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::CountrySelection);
}
