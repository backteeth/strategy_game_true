use crate::app::game_state::GameState;
use crate::country::{CountryData, CountryRegistry};
use crate::state::data::{StateData, StateRegistry};
use bevy::prelude::*;
/// データローダーモジュール
/// assets/data/ 以下の RON ファイルを起動時に読み込み、バリデーションを行う
use std::collections::HashSet;

/// ローダープラグイン
/// Startup システムでRONを読み込み、バリデーション後にResourceへ注入する
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
/// 失敗時はパニックして原因を表示する（起動時エラーは致命的）
pub fn load_game_data(
    mut country_registry: ResMut<CountryRegistry>,
    mut state_registry: ResMut<StateRegistry>,
) {
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

    let states: Vec<StateData> = ron::from_str(&states_ron)
        .unwrap_or_else(|e| panic!("[DataLoader] Failed to parse assets/data/states.ron: {e}"));

    // ── バリデーション ───────────────────────────────────────────────────
    validate_data(&countries, &states);

    info!(
        "[DataLoader] Loaded {} countries, {} states",
        countries.len(),
        states.len()
    );

    // ── Resource に注入 ──────────────────────────────────────────────────
    country_registry.countries = countries;
    *state_registry = StateRegistry::build(states);
}

/// データの整合性を検証する
/// 問題があれば panic して詳細メッセージを表示する
fn validate_data(countries: &[CountryData], states: &[StateData]) {
    // CountryId 重複チェック
    let mut country_ids = HashSet::new();
    for c in countries {
        if !country_ids.insert(c.id.0) {
            panic!("[DataLoader] Duplicate CountryId: {}", c.id.0);
        }
    }

    // StateId 重複チェック
    let mut state_ids = HashSet::new();
    for s in states {
        if !state_ids.insert(s.id.0) {
            panic!("[DataLoader] Duplicate StateId: {}", s.id.0);
        }
    }

    // 州の所有国が存在するか
    for s in states {
        if !country_ids.contains(&s.owner_country_id.0) {
            panic!(
                "[DataLoader] State '{}' (id={}) references unknown CountryId: {}",
                s.name, s.id.0, s.owner_country_id.0
            );
        }
    }

    // 国家の首都州が存在し、かつその国家が所有しているか
    for c in countries {
        if !state_ids.contains(&c.capital_state_id.0) {
            panic!(
                "[DataLoader] Country '{}' (id={}) references unknown capital StateId: {}",
                c.name, c.id.0, c.capital_state_id.0
            );
        }
        // 首都州の所有者確認
        let capital_owner = states
            .iter()
            .find(|s| s.id == c.capital_state_id)
            .map(|s| s.owner_country_id);

        if capital_owner != Some(c.id) {
            panic!(
                "[DataLoader] Country '{}' (id={}) capital state {} is not owned by that country \
                 (owner: {:?})",
                c.name, c.id.0, c.capital_state_id.0, capital_owner
            );
        }
    }
}

/// データ読み込み完了後に CountrySelection へ遷移する
pub fn transition_to_country_selection(mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::CountrySelection);
}
