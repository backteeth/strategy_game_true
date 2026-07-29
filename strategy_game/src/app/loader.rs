use crate::app::game_state::GameState;
use crate::building::data::{BuildingDefinition, BuildingRegistry, BuildingType};
use crate::common::StateId;
use crate::country::{CountryData, CountryRegistry};
use crate::diplomacy::data::{
    ActiveTreaty, DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation,
    InitialDiplomaticRelation,
};
use crate::economy::resources::StateResourceDeposit;
use crate::military::data::{ArmyStatus, ArmyUnit, DivisionDefinition, MilitaryRegistry};
use crate::research::data::{TechnologyDefinition, TechnologyRegistry};
use crate::research::world_stage::{WorldCivilizationState, WorldStageDefinition};
use crate::state::data::{StateData, StateRegistry};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// ローダープラグイン
pub struct DataLoaderPlugin;

impl Plugin for DataLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            load_game_data.before(crate::app::loader::transition_to_country_selection),
        )
        .add_systems(Startup, transition_to_country_selection)
        .add_systems(OnEnter(GameState::Playing), spawn_debug_armies);
    }
}

/// RONファイルからゲームデータを読み込む
pub fn load_game_data(
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
    mut building_registry: ResMut<BuildingRegistry>,
    mut tech_registry: ResMut<TechnologyRegistry>,
    mut world_state: ResMut<WorldCivilizationState>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
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
        building_map.insert(b.building_type, b);
    }
    building_registry.definitions = building_map;

    // ── 技術データ読み込み ───────────────────────────────────────────────
    let tech_ron = std::fs::read_to_string("assets/data/technologies.ron").unwrap_or_else(|e| {
        panic!("[DataLoader] Failed to read assets/data/technologies.ron: {e}")
    });

    let tech_defs: Vec<TechnologyDefinition> = ron::from_str(&tech_ron).unwrap_or_else(|e| {
        panic!("[DataLoader] Failed to parse assets/data/technologies.ron: {e}")
    });

    let mut tech_map = HashMap::new();
    let mut sorted_ids = Vec::new();

    for t in tech_defs {
        if t.cost <= 0.0 {
            panic!(
                "[DataLoader] Technology '{}' has invalid cost (<= 0): {}",
                t.id, t.cost
            );
        }
        if tech_map.contains_key(&t.id) {
            panic!("[DataLoader] Duplicate Technology ID: '{}'", t.id);
        }
        sorted_ids.push(t.id.clone());
        tech_map.insert(t.id.clone(), t);
    }
    tech_registry.definitions = tech_map;
    tech_registry.sorted_ids = sorted_ids;

    // ── 世界文明段階データ読み込み ─────────────────────────────────────────
    let stages_ron = std::fs::read_to_string("assets/data/world_stages.ron").unwrap_or_else(|e| {
        panic!("[DataLoader] Failed to read assets/data/world_stages.ron: {e}")
    });

    let stage_defs: Vec<WorldStageDefinition> = ron::from_str(&stages_ron).unwrap_or_else(|e| {
        panic!("[DataLoader] Failed to parse assets/data/world_stages.ron: {e}")
    });

    let mut stage_map = HashMap::new();
    for s in stage_defs {
        if s.required_country_count == 0 {
            panic!(
                "[DataLoader] World stage '{:?}' has required_country_count of 0",
                s.stage
            );
        }
        stage_map.insert(s.stage, s);
    }
    world_state.stage_definitions = stage_map;

    // ── 外交初期データ読み込み ─────────────────────────────────────────────
    if let Ok(diplo_ron) = std::fs::read_to_string("assets/data/diplomacy.ron")
        && let Ok(initial_diplomacy) = ron::from_str::<Vec<InitialDiplomaticRelation>>(&diplo_ron)
    {
        for init in initial_diplomacy {
            if let Some(key) = DiplomaticPairKey::new(init.country_a, init.country_b) {
                let mut rel = DiplomaticRelation {
                    opinion: init.opinion.clamp(-100.0, 100.0),
                    tension: init.tension.clamp(0.0, 100.0),
                    trust: init.trust.clamp(0.0, 100.0),
                    has_military_access: init.has_military_access,
                    ..Default::default()
                };

                let mut treaties = init.treaties.clone();
                if init.alliance
                    && !treaties.contains(&crate::diplomacy::data::TreatyType::Alliance)
                {
                    treaties.push(crate::diplomacy::data::TreatyType::Alliance);
                }

                for t_type in treaties {
                    rel.treaties.push(ActiveTreaty {
                        treaty_type: t_type,
                        countries: (init.country_a, init.country_b),
                        signed_date: "1800/01/01".to_string(),
                        is_active: true,
                    });
                }
                diplomacy_registry.relations.insert(key, rel);
            }
        }
    }

    // ── 資源鉱床データ読み込み ───────────────────────────────────────────
    let resources_ron = std::fs::read_to_string("assets/data/resources.ron")
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to read assets/data/resources.ron: {e}"));

    let resource_deposits_map: HashMap<StateId, Vec<StateResourceDeposit>> =
        ron::from_str(&resources_ron).unwrap_or_else(|e| {
            panic!("[DataLoader] Failed to parse assets/data/resources.ron: {e}")
        });

    // ── 師団データ読み込み ───────────────────────────────────────────────
    let divisions_ron = std::fs::read_to_string("assets/data/divisions.ron")
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to read assets/data/divisions.ron: {e}"));

    let division_defs: Vec<DivisionDefinition> = ron::from_str(&divisions_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/divisions.ron: {e}"));

    let mut div_map = HashMap::new();
    for d in division_defs {
        if div_map.contains_key(&d.id) {
            panic!("[DataLoader] Duplicate Division ID: '{:?}'", d.id);
        }
        div_map.insert(d.id, d);
    }
    military_registry.definitions = div_map;

    // ── 国家データ読み込み ───────────────────────────────────────────────
    let countries_ron = std::fs::read_to_string("assets/data/countries.ron")
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to read assets/data/countries.ron: {e}"));

    let countries: Vec<CountryData> = ron::from_str(&countries_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/countries.ron: {e}"));

    // ── 州データ読み込み ─────────────────────────────────────────────────
    let states_ron = std::fs::read_to_string("assets/data/states.ron")
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to read assets/data/states.ron: {e}"));

    let mut states: Vec<StateData> = ron::from_str(&states_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/states.ron: {e}"));

    for state in states.iter_mut() {
        if let Some(deposits) = resource_deposits_map.get(&state.id) {
            state.resource_deposits = deposits.clone();
        }
    }

    validate_data(
        &countries,
        &states,
        &building_registry.definitions,
        &tech_registry.definitions,
    );

    info!(
        "[DataLoader] Successfully loaded {} buildings, {} technologies, {} countries, {} states, {} diplomatic relations",
        building_registry.definitions.len(),
        tech_registry.definitions.len(),
        countries.len(),
        states.len(),
        diplomacy_registry.relations.len()
    );

    country_registry.countries = countries;
    *state_registry = StateRegistry::build(states);
}

fn validate_data(
    countries: &[CountryData],
    states: &[StateData],
    building_defs: &HashMap<BuildingType, BuildingDefinition>,
    tech_defs: &HashMap<String, TechnologyDefinition>,
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

    for (tech_id, def) in tech_defs {
        for pre in &def.prerequisites {
            if !tech_defs.contains_key(pre) {
                panic!(
                    "[DataLoader] Technology '{}' references non-existent prerequisite: '{}'",
                    tech_id, pre
                );
            }
        }
    }

    for s in states {
        if !country_ids.contains(&s.owner_country_id.0) {
            panic!(
                "[DataLoader] State '{}' (id={}) references unknown CountryId: {}",
                s.name, s.id.0, s.owner_country_id.0
            );
        }
        for (&b_type, &lvl) in &s.buildings {
            if let Some(def) = building_defs.get(&b_type) {
                if lvl > def.max_level {
                    panic!(
                        "[DataLoader] State '{}' (id={}) building level for {:?} ({}) exceeds max level ({})",
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
}

pub fn transition_to_country_selection(mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::CountrySelection);
}

/// ゲーム開始時にデバッグ用の部隊を各国首都に1部隊ずつ配置する
pub fn spawn_debug_armies(
    mut military_registry: ResMut<MilitaryRegistry>,
    country_registry: Res<crate::country::CountryRegistry>,
) {
    // Infantry (DivisionId=0) を各国首都に配置
    let infantry_def_id = crate::common::DivisionId(0);

    for country in country_registry.countries.iter() {
        if let Some(def) = military_registry.definitions.get(&infantry_def_id) {
            let def_id = infantry_def_id;
            let new_army = ArmyUnit {
                id: crate::common::ArmyId(0), // add_army で上書きされる
                owner: country.id,
                division_type: def.division_type,
                size: def.size,
                current_state: country.capital_state_id,
                destination: None,
                current_path: Vec::new(),
                target_state: None,
                manpower: def.required_manpower,
                max_manpower: def.required_manpower,
                equipment: def.required_equipment,
                max_equipment: def.required_equipment,
                organization: def.organization,
                max_organization: def.organization,
                morale: def.morale,
                max_morale: def.morale,
                experience: 0.0,
                supply_ratio: 1.0,
                movement_progress: 0.0,
                status: ArmyStatus::Idle,
                def_id,
            };
            military_registry.add_army(new_army);
        }
    }

    info!(
        "[DEBUG] Spawned {} initial armies",
        military_registry.armies.len()
    );
}
