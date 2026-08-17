/// P21-009-FIX-001: 既存Mine配置(初期データ・旧セーブ双方)をCrystalMineへ正規化する。
///
/// P21-009でMineの鉱床走査からMagicCrystal種別を除外した結果、MagicCrystal鉱床しか
/// 持たない州(州2「Western Mage Province」・州7「Eastern Technology」)に既に配置されて
/// いたMineが無出力になった。本モジュールはそのMineレベルを同数のCrystalMineへ
/// 一度だけ正規化する(通常Mineから直接MagicCrystal/RawMagicCrystalを生産する仕様には
/// 戻さない)。
///
/// 適用対象は「クリスタル専用州」(discoveredなMagicCrystal鉱床を持ち、他のdiscovered
/// 鉱床を持たない州、`economy::resources::is_crystal_only_state`で判定)に限る。
/// MagicCrystalと他資源が同居する「混合鉱床州」は自動変換せず、`skipped_mixed_deposit_states`
/// へ記録して報告する(2026-08-16時点の`resources.ron`には混合鉱床州は存在しないが、
/// 将来データが追加された場合に誤って自動変換しないための安全弁)。
use crate::building::construction::ConstructionQueueItem;
use crate::building::data::{BuildingDefinition, BuildingType};
use crate::common::StateId;
use crate::country::CountryData;
use crate::economy::resources::is_crystal_only_state;
use crate::state::data::StateData;
use std::collections::{HashMap, HashSet};

/// 移行結果の要約(呼び出し側でのログ出力・テスト検証用)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MineMigrationReport {
    /// (StateId, 移行したMineレベル)。レベル0(=対象Mineが存在しない)の州は含まない。
    pub migrated_states: Vec<(StateId, u32)>,
    /// MagicCrystalと他資源鉱床が同居するため自動変換をスキップした州。
    pub skipped_mixed_deposit_states: Vec<StateId>,
    /// building_typeをMineからCrystalMineへ変換した建設中Projectの件数。
    pub migrated_construction_items: usize,
}

impl MineMigrationReport {
    pub fn is_empty(&self) -> bool {
        self.migrated_states.is_empty()
            && self.skipped_mixed_deposit_states.is_empty()
            && self.migrated_construction_items == 0
    }
}

/// 州群・国家群の両方に対して、クリスタル専用州のMineをCrystalMineへ移行する。
///
/// 同じデータに対して複数回呼び出しても結果が増殖しない(冪等): 1回目の移行で
/// `buildings`からMineキーが除去されるため、2回目以降は移行対象レベルが0となり
/// 何も変化しない。既にCrystalMineが存在する州では、そのレベルへ加算する
/// (新規に上書きしない)。
pub fn migrate_mines_to_crystal_mines(
    states: &mut [StateData],
    countries: &mut [CountryData],
    building_definitions: &HashMap<BuildingType, BuildingDefinition>,
) -> MineMigrationReport {
    let mut report = MineMigrationReport::default();

    // クリスタル専用州と、混合鉱床のため対象外とする州を先に分類する
    // (StateId昇順で報告が決定的になるよう、Vec<StateId>を集めた後にソートする)。
    let mut crystal_only_states: HashSet<StateId> = HashSet::new();
    let mut mixed_states: Vec<StateId> = Vec::new();
    for state in states.iter() {
        let has_crystal = crate::economy::resources::has_discovered_deposit(
            &state.resource_deposits,
            crate::economy::resources::ResourceType::MagicCrystal,
        );
        if !has_crystal {
            continue;
        }
        if is_crystal_only_state(&state.resource_deposits) {
            crystal_only_states.insert(state.id);
        } else {
            mixed_states.push(state.id);
        }
    }
    mixed_states.sort_by_key(|id| id.0);
    report.skipped_mixed_deposit_states = mixed_states;

    // 完成済み建物レベル(state.buildings)の移行。StateId昇順で決定的に処理する。
    let mut state_indices: Vec<usize> = (0..states.len()).collect();
    state_indices.sort_by_key(|&i| states[i].id.0);
    for i in state_indices {
        let state = &mut states[i];
        if !crystal_only_states.contains(&state.id) {
            continue;
        }
        let mine_level = state.buildings.remove(&BuildingType::Mine).unwrap_or(0);
        if mine_level == 0 {
            continue;
        }
        let crystal_level = state
            .buildings
            .entry(BuildingType::CrystalMine)
            .or_insert(0);
        *crystal_level += mine_level;
        report.migrated_states.push((state.id, mine_level));
    }

    // 建設中Project(construction_queue)の移行。完成割合(progress/required_progress)は
    // 変えず、required_progressだけをCrystalMineの正規値へ換算する。
    if let Some(crystal_mine_def) = building_definitions.get(&BuildingType::CrystalMine) {
        let new_required = crystal_mine_def.required_progress;
        for country in countries.iter_mut() {
            for item in country.construction_queue.iter_mut() {
                if item.building_type == BuildingType::Mine
                    && crystal_only_states.contains(&item.state_id)
                {
                    migrate_construction_item(item, new_required);
                    report.migrated_construction_items += 1;
                }
            }
        }
    }

    report
}

fn migrate_construction_item(item: &mut ConstructionQueueItem, new_required_progress: f64) {
    let fraction = if item.required_progress > 0.0 {
        (item.progress / item.required_progress).clamp(0.0, 1.0)
    } else {
        0.0
    };
    item.building_type = BuildingType::CrystalMine;
    item.required_progress = new_required_progress;
    item.progress = fraction * new_required_progress;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::construction::ConstructionStatus;
    use crate::common::CountryId;
    use crate::economy::resources::{ResourceType, StateResourceDeposit};

    fn crystal_deposit() -> StateResourceDeposit {
        StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        }
    }

    fn iron_deposit() -> StateResourceDeposit {
        StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: true,
            development_level: 1,
        }
    }

    fn crystal_only_state(id: StateId, mine_level: u32) -> StateData {
        let mut buildings = HashMap::new();
        if mine_level > 0 {
            buildings.insert(BuildingType::Mine, mine_level);
        }
        StateData {
            id,
            resource_deposits: vec![crystal_deposit()],
            buildings,
            ..Default::default()
        }
    }

    fn building_defs() -> HashMap<BuildingType, BuildingDefinition> {
        let mut defs = HashMap::new();
        defs.insert(
            BuildingType::CrystalMine,
            BuildingDefinition {
                building_type: BuildingType::CrystalMine,
                name: "Crystal Mine".to_string(),
                construction_cost: 550.0,
                required_progress: 60.0,
                required_workforce: 10_000.0,
                logistics_cost: 10.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 18.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        defs
    }

    /// 要求テスト項目2: 初期CrystalMineレベルが旧Mineレベルと一致する。
    #[test]
    fn completed_mine_level_migrates_to_crystal_mine_one_to_one() {
        let mut states = vec![crystal_only_state(StateId(2), 1)];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        let report = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert_eq!(states[0].building_level(BuildingType::Mine), 0);
        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 1);
        assert_eq!(report.migrated_states, vec![(StateId(2), 1)]);
    }

    /// 要求テスト項目1: 新規ゲームの州2・7に無稼働Mineが残らない(0件でも複数州対応)。
    #[test]
    fn multiple_crystal_only_states_all_migrate() {
        let mut states = vec![
            crystal_only_state(StateId(2), 1),
            crystal_only_state(StateId(7), 2),
        ];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        for state in &states {
            assert_eq!(state.building_level(BuildingType::Mine), 0);
        }
        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 1);
        assert_eq!(states[1].building_level(BuildingType::CrystalMine), 2);
    }

    /// 要求テスト項目4: 既存CrystalMineがある場合は安全に加算される(上書きしない)。
    #[test]
    fn existing_crystal_mine_level_is_added_to_not_overwritten() {
        let mut state = crystal_only_state(StateId(2), 1);
        state.buildings.insert(BuildingType::CrystalMine, 3);
        let mut states = vec![state];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 4);
    }

    /// 要求テスト項目5: 同じデータへ複数回移行を適用しても結果が増殖しない(冪等)。
    #[test]
    fn migration_is_idempotent_across_repeated_application() {
        let mut states = vec![crystal_only_state(StateId(2), 1)];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        let first = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);
        let second = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 1);
        assert_eq!(states[0].building_level(BuildingType::Mine), 0);
        assert_eq!(first.migrated_states, vec![(StateId(2), 1)]);
        assert!(
            second.migrated_states.is_empty(),
            "second pass must migrate nothing further, got {:?}",
            second.migrated_states
        );
    }

    /// 要求テスト項目6: 建設中Projectの種類がCrystalMineへ変換され、完成割合が維持される。
    #[test]
    fn in_progress_construction_item_converts_and_preserves_completion_fraction() {
        let mut states = vec![crystal_only_state(StateId(2), 0)];
        let mut countries = vec![CountryData {
            id: CountryId(0),
            construction_queue: vec![ConstructionQueueItem {
                state_id: StateId(2),
                building_type: BuildingType::Mine,
                target_level: 1,
                progress: 30.0,          // 旧Mine required_progress=60.0の50%
                required_progress: 60.0, // 旧Mine定義値(テストではDBに登録せず直接指定)
                paid_cost: 500.0,
                status: ConstructionStatus::InProgress,
            }],
            ..CountryData::default()
        }];
        let defs = building_defs(); // CrystalMine.required_progress = 60.0

        let report = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        let item = &countries[0].construction_queue[0];
        assert_eq!(item.building_type, BuildingType::CrystalMine);
        assert_eq!(item.required_progress, 60.0);
        assert!(
            (item.progress - 30.0).abs() < 1e-9,
            "50% completion at old required_progress=60 must remain 50% (30.0) at new required_progress=60, got {}",
            item.progress
        );
        assert_eq!(item.paid_cost, 500.0, "paid_cost must not be altered");
        assert_eq!(item.target_level, 1, "target_level must not be altered");
        assert_eq!(report.migrated_construction_items, 1);
    }

    /// required_progressが異なる場合でも完成割合(%)が維持されることを確認する
    /// (旧Mine required_progress=100、新CrystalMine required_progress=60のケース)。
    #[test]
    fn construction_item_completion_fraction_is_preserved_across_differing_required_progress() {
        let mut states = vec![crystal_only_state(StateId(2), 0)];
        let mut countries = vec![CountryData {
            id: CountryId(0),
            construction_queue: vec![ConstructionQueueItem {
                state_id: StateId(2),
                building_type: BuildingType::Mine,
                target_level: 1,
                progress: 25.0,
                required_progress: 100.0, // 25%完成
                paid_cost: 500.0,
                status: ConstructionStatus::InProgress,
            }],
            ..CountryData::default()
        }];
        let defs = building_defs(); // CrystalMine.required_progress = 60.0

        migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        let item = &countries[0].construction_queue[0];
        assert_eq!(item.required_progress, 60.0);
        assert!(
            (item.progress - 15.0).abs() < 1e-9, // 25% of 60.0
            "25% completion must be preserved at the new required_progress, got {}",
            item.progress
        );
    }

    /// 混合鉱床州(MagicCrystal + Iron等)は自動変換せず、報告のみ行う。
    #[test]
    fn mixed_deposit_state_is_skipped_and_reported_not_migrated() {
        let mut state = StateData {
            id: StateId(3),
            resource_deposits: vec![crystal_deposit(), iron_deposit()],
            ..Default::default()
        };
        state.buildings.insert(BuildingType::Mine, 2);
        let mut states = vec![state];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        let report = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert_eq!(
            states[0].building_level(BuildingType::Mine),
            2,
            "mixed-deposit state's Mine must not be touched"
        );
        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 0);
        assert_eq!(report.skipped_mixed_deposit_states, vec![StateId(3)]);
        assert!(report.migrated_states.is_empty());
    }

    /// 要求テスト項目8: Iron/Coalのみの州(クリスタル鉱床なし)は完全に対象外。
    #[test]
    fn non_crystal_state_is_entirely_unaffected() {
        let mut state = StateData {
            id: StateId(0),
            resource_deposits: vec![iron_deposit()],
            ..Default::default()
        };
        state.buildings.insert(BuildingType::Mine, 5);
        let mut states = vec![state];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        let report = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert_eq!(states[0].building_level(BuildingType::Mine), 5);
        assert_eq!(states[0].building_level(BuildingType::CrystalMine), 0);
        assert!(report.migrated_states.is_empty());
        assert!(report.skipped_mixed_deposit_states.is_empty());
    }

    #[test]
    fn no_mine_present_produces_no_report_entries() {
        let mut states = vec![crystal_only_state(StateId(2), 0)];
        let mut countries: Vec<CountryData> = Vec::new();
        let defs = building_defs();

        let report = migrate_mines_to_crystal_mines(&mut states, &mut countries, &defs);

        assert!(report.is_empty());
    }
}
