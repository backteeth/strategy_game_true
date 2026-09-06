/// P26-004-PRE-004: Elfin Republic(StateId 16)/Ferrowyn League(StateId 21)/
/// Passguard March(StateId 13)/Oceanic Magic Empire(StateId 8)へ、最小限の
/// MilitarySupplies生産網(Iron/Coal鉱床、OceanicのみMilitaryFactory level 1)を、
/// New Game・旧Save双方へ決定的・冪等・原子的に適用する構造的migration。
///
/// 採用パッケージの根拠は`verification_logs/phase-26/p26-004-pre-003/
/// p26-004-pre-003_minimum_supply_chains_and_initialization_design.md`§17〜18を参照
/// (Passguardの2鉱床を同一州に集約する理由=`country_ai.rs`のMine候補州選定が
/// 「不足資源ごとの区別」を持たないための回避、Oceanicのみ建物追加が必要な理由=
/// 鉱床追加だけではMilitaryFactory自体が存在しないため)。
///
/// ## 冪等性の根拠(マーカーフィールド不要、構造チェックのみで十分と判断した理由)
///
/// 2026-09-06時点で`src/`全体を`rg`監査した結果:
/// - `StateData.resource_deposits`(`Vec<StateResourceDeposit>`)へ要素を追加・削除する
///   経路は、本番コード(テスト・本migration自身を除く)に一切存在しない
///   (`resource_deposits.push/remove/retain/clear`は0件)。唯一の書き込み元は
///   `src/app/loader.rs`の初期RON読込(`resources.ron`をまるごと代入)であり、動的な
///   鉱床の発見・枯渇・除去システムは存在しない。
/// - `StateData.buildings`(`HashMap<BuildingType, u32>`)のレベルを引き下げる、または
///   キーを削除する経路も本番コードに存在しない。書き込み元は
///   `src/economy/mod.rs`(建設完了時に`current + 1`で単調増加させるのみ)と
///   `src/building/mine_migration.rs`(Mine→CrystalMineへの同数移し替え、本migrationの
///   対象[MilitaryFactory]とは無関係な建物種別)の2箇所のみ。
///
/// したがって「対象StateIdに、追加しようとしている`resource_type`のdiscovered鉱床が
/// 既に1件でも存在するか」「対象の建物レベルが既に目標水準以上か」という**構造的な
/// 判定だけ**で、複数回の適用に対して安全に冪等となる(`mine_migration`と同じ設計
/// パターン)。プレイヤーが鉱床・MilitaryFactoryを削除して再ロードする経路が
/// 存在しないため、新規のbool marker(`#[serde(default)]`)は不要と判断した——
/// 将来「鉱床探索」「建物解体」等の新規ゲームプレイ機構が追加された場合は、
/// この前提を必ず再監査すること。
use crate::building::data::BuildingType;
use crate::common::StateId;
use crate::economy::resources::{ResourceType, StateResourceDeposit, has_discovered_deposit};
use crate::state::data::StateData;

/// 1件の鉱床配置ターゲット。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepositPlacement {
    pub state_id: StateId,
    pub resource_type: ResourceType,
    pub base_output: f64,
    pub development_level: u32,
}

/// 1件の建物レベル下限ターゲット(「既にlevel N以上であること」を保証する、
/// 加算ではなく下限保証)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildingLevelFloor {
    pub state_id: StateId,
    pub building_type: BuildingType,
    pub min_level: u32,
}

/// `verification_logs/phase-26/p26-004-pre-003/`§17〜18で確定した固定パッケージ:
/// Elfin(16)/Ferrowyn(21)/Passguard(13)/Oceanic(8)へそれぞれ
/// Iron(base_output:50.0)・Coal(base_output:40.0)、discovered:true、development_level:1。
/// Passguardは意図的にIron・Coal両方を同一StateId(13)へ配置する(country_ai.rsの
/// Mine候補州選定が資源種別を区別しないための回避、上記モジュールdocを参照)。
pub fn minimum_supply_chain_deposit_placements() -> Vec<DepositPlacement> {
    const TARGET_STATES: [StateId; 4] = [StateId(16), StateId(21), StateId(13), StateId(8)];
    let mut placements = Vec::with_capacity(TARGET_STATES.len() * 2);
    for state_id in TARGET_STATES {
        placements.push(DepositPlacement {
            state_id,
            resource_type: ResourceType::Iron,
            base_output: 50.0,
            development_level: 1,
        });
        placements.push(DepositPlacement {
            state_id,
            resource_type: ResourceType::Coal,
            base_output: 40.0,
            development_level: 1,
        });
    }
    placements
}

/// Oceanic(StateId 8、帝都)へMilitaryFactoryが最低level 1であることを保証する。
/// 鉱床追加だけでは生産手段自体が存在しないため必要(§16実測)。
pub fn minimum_supply_chain_building_floors() -> Vec<BuildingLevelFloor> {
    vec![BuildingLevelFloor {
        state_id: StateId(8),
        building_type: BuildingType::MilitaryFactory,
        min_level: 1,
    }]
}

/// migrationが対象StateIdの欠落を検出した場合のエラー(原子性: この場合`states`は
/// 一切変更されない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingTargetState(pub StateId);

/// 適用結果の要約(呼び出し側のログ・テスト検証用)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SupplyChainMigrationReport {
    /// 新規に追加した鉱床(StateId, ResourceType)。既存鉱床を保護するため、既に
    /// discovered済みの同種鉱床がある州は含まれない。
    pub added_deposits: Vec<(StateId, ResourceType)>,
    /// 新規に引き上げた建物レベル(StateId, BuildingType, 設定後level)。既に
    /// min_level以上だった州は含まれない。
    pub raised_buildings: Vec<(StateId, BuildingType, u32)>,
}

impl SupplyChainMigrationReport {
    pub fn is_empty(&self) -> bool {
        self.added_deposits.is_empty() && self.raised_buildings.is_empty()
    }
}

/// P26-004-PRE-004の固定パッケージを`states`へ原子的・冪等に適用する。
///
/// 手順(`verification_logs/phase-26/p26-004-pre-004/`spec §6の6段階に対応):
/// 1. 対象StateId(8/13/16/21)の集合のうち、`states`に**1件も**存在しない場合は、
///    「この`states`はそもそも本タスクが対象とする地図ではない」と判断し、
///    何も変更せず空の`Ok`を返す。これは本関数が`src/app/loader.rs`・
///    `src/save/apply.rs`という共有インフラ経路から無条件に呼ばれるため
///    (`mine_migration`と同じ配線)、この2箇所を経由する既存の大量の単体テスト
///    (本番の6か国地図とは無関係な、State(0)/State(1)等の小規模テスト専用地図を
///    使うもの)を、本タスクと無関係な理由で壊さないために必須の分岐である。
/// 2. 対象StateIdのうち**1件以上**が`states`に存在するが、**全件ではない**場合
///    (=本番の地図に近いが一部が欠落・破損している状態)は、**どのstateも変更する
///    前に**それを検出し、即座に`Err`を返して`states`を一切変更しない(原子性、
///    spec §6 Test G)。
/// 3. 対象StateIdが全て存在する場合のみ、各placement/floorについて構造的な冪等条件
///    ("discovered済みの同種鉱床が既にあるか"/"建物レベルが既にmin_level以上か")を
///    判定するだけの、失敗し得ない純粋なデータ変換を行う(`mine_migration`と同じ
///    設計方針)。
///
/// 複数回呼び出しても結果が増殖しない: 1回目の適用後は判定条件そのものが真になるため、
/// 2回目以降は何も追加しない(`tests::applying_twice_is_idempotent`で確認)。
/// `states`のスライス内順序(HashMap由来の非決定順を含む)には一切依存しない
/// (StateId昇順に並べ替えてから処理する)。
pub fn apply_minimum_supply_chain_migration(
    states: &mut [StateData],
) -> Result<SupplyChainMigrationReport, MissingTargetState> {
    let deposit_placements = minimum_supply_chain_deposit_placements();
    let building_floors = minimum_supply_chain_building_floors();

    let mut target_ids: Vec<StateId> = deposit_placements
        .iter()
        .map(|p| p.state_id)
        .chain(building_floors.iter().map(|f| f.state_id))
        .collect();
    target_ids.sort_by_key(|id| id.0);
    target_ids.dedup();

    // 0. この`states`が対象StateIdを1件も含まない場合、本タスク対象外の地図として
    //    何もせず成功扱いにする(上記doc注記を参照)。
    let present_count = target_ids
        .iter()
        .filter(|id| states.iter().any(|s| s.id == **id))
        .count();
    if present_count == 0 {
        return Ok(SupplyChainMigrationReport::default());
    }

    // 1. 検証専用パス: 1件以上は存在するが全件ではない場合、どのstateも変更せず
    //    即座に拒否する(原子性)。
    for placement in &deposit_placements {
        if !states.iter().any(|s| s.id == placement.state_id) {
            return Err(MissingTargetState(placement.state_id));
        }
    }
    for floor in &building_floors {
        if !states.iter().any(|s| s.id == floor.state_id) {
            return Err(MissingTargetState(floor.state_id));
        }
    }

    // 2. 全対象の存在が確認できたので、ここから先は失敗し得ない純粋な変換のみを行う。
    let mut report = SupplyChainMigrationReport::default();

    // StateId昇順で決定的に処理する(HashMap由来の非決定順に依存しない)。
    let mut indices: Vec<usize> = (0..states.len()).collect();
    indices.sort_by_key(|&i| states[i].id.0);

    for &i in &indices {
        let state = &mut states[i];
        for placement in &deposit_placements {
            if placement.state_id != state.id {
                continue;
            }
            if has_discovered_deposit(&state.resource_deposits, placement.resource_type) {
                continue; // 既存の鉱床を保護(重複追加・上書きしない)
            }
            state.resource_deposits.push(StateResourceDeposit {
                resource_type: placement.resource_type,
                base_output: placement.base_output,
                discovered: true,
                development_level: placement.development_level,
            });
            report
                .added_deposits
                .push((state.id, placement.resource_type));
        }
        for floor in &building_floors {
            if floor.state_id != state.id {
                continue;
            }
            let current = state.building_level(floor.building_type);
            if current >= floor.min_level {
                continue; // 既存レベルを引き下げない・加算しない
            }
            state.buildings.insert(floor.building_type, floor.min_level);
            report
                .raised_buildings
                .push((state.id, floor.building_type, floor.min_level));
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn bare_state(id: usize) -> StateData {
        StateData {
            id: StateId(id),
            ..Default::default()
        }
    }

    fn target_states() -> Vec<StateData> {
        vec![
            bare_state(8),
            bare_state(13),
            bare_state(16),
            bare_state(21),
        ]
    }

    fn deposit_of(
        state: &StateData,
        resource_type: ResourceType,
    ) -> Option<&StateResourceDeposit> {
        state
            .resource_deposits
            .iter()
            .find(|d| d.resource_type == resource_type)
    }

    // ─── 新規適用(Test B相当): 何も持たない州へ正しく追加される ──────────

    #[test]
    fn fresh_states_receive_iron_and_coal_deposits_on_all_four_target_states() {
        let mut states = target_states();
        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert_eq!(report.added_deposits.len(), 8, "4 states x 2 resources");
        for id in [8usize, 13, 16, 21] {
            let state = states.iter().find(|s| s.id == StateId(id)).unwrap();
            let iron = deposit_of(state, ResourceType::Iron).unwrap();
            assert_eq!(iron.base_output, 50.0);
            assert!(iron.discovered);
            assert_eq!(iron.development_level, 1);
            let coal = deposit_of(state, ResourceType::Coal).unwrap();
            assert_eq!(coal.base_output, 40.0);
            assert!(coal.discovered);
            assert_eq!(coal.development_level, 1);
        }
    }

    #[test]
    fn oceanic_state_8_receives_military_factory_level_1() {
        let mut states = target_states();
        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert_eq!(
            report.raised_buildings,
            vec![(StateId(8), BuildingType::MilitaryFactory, 1)]
        );
        let oceanic = states.iter().find(|s| s.id == StateId(8)).unwrap();
        assert_eq!(oceanic.building_level(BuildingType::MilitaryFactory), 1);
    }

    #[test]
    fn non_target_states_are_left_completely_untouched() {
        let mut states = target_states();
        states.push(bare_state(0)); // Arcadia相当、対象外
        let mut arcadia_farm = bare_state(0);
        arcadia_farm.buildings.insert(BuildingType::Farm, 3);
        states.pop();
        states.push(arcadia_farm);

        apply_minimum_supply_chain_migration(&mut states).unwrap();

        let arcadia = states.iter().find(|s| s.id == StateId(0)).unwrap();
        assert!(arcadia.resource_deposits.is_empty());
        assert_eq!(arcadia.building_level(BuildingType::Farm), 3);
        assert_eq!(arcadia.building_level(BuildingType::MilitaryFactory), 0);
    }

    // ─── 部分適用済み(Test C相当) ───────────────────────────────────

    #[test]
    fn state_with_only_iron_gets_only_coal_added_and_iron_value_is_untouched() {
        let mut states = target_states();
        let elfin = states.iter_mut().find(|s| s.id == StateId(16)).unwrap();
        elfin.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 999.0, // 実運用ではあり得ない値だが、上書きされないことの検証用
            discovered: true,
            development_level: 7,
        });

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        let elfin_additions: Vec<_> = report
            .added_deposits
            .iter()
            .filter(|(id, _)| *id == StateId(16))
            .cloned()
            .collect();
        assert_eq!(elfin_additions, vec![(StateId(16), ResourceType::Coal)]);
        let elfin = states.iter().find(|s| s.id == StateId(16)).unwrap();
        let iron = deposit_of(elfin, ResourceType::Iron).unwrap();
        assert_eq!(iron.base_output, 999.0, "existing Iron value must not be overwritten");
        assert_eq!(iron.development_level, 7);
        assert_eq!(
            elfin
                .resource_deposits
                .iter()
                .filter(|d| d.resource_type == ResourceType::Iron)
                .count(),
            1,
            "must not duplicate an already-present Iron deposit"
        );
        assert!(deposit_of(elfin, ResourceType::Coal).is_some());
    }

    #[test]
    fn state_with_only_coal_gets_only_iron_added() {
        let mut states = target_states();
        let ferrowyn = states.iter_mut().find(|s| s.id == StateId(21)).unwrap();
        ferrowyn.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::Coal,
            base_output: 40.0,
            discovered: true,
            development_level: 1,
        });

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        let ferrowyn_additions: Vec<_> = report
            .added_deposits
            .iter()
            .filter(|(id, _)| *id == StateId(21))
            .cloned()
            .collect();
        assert_eq!(
            ferrowyn_additions,
            vec![(StateId(21), ResourceType::Iron)]
        );
    }

    #[test]
    fn state_with_both_deposits_already_present_adds_nothing() {
        let mut states = target_states();
        let passguard = states.iter_mut().find(|s| s.id == StateId(13)).unwrap();
        passguard.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 50.0,
            discovered: true,
            development_level: 1,
        });
        passguard.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::Coal,
            base_output: 40.0,
            discovered: true,
            development_level: 1,
        });

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        let passguard_additions: Vec<_> = report
            .added_deposits
            .iter()
            .filter(|(id, _)| *id == StateId(13))
            .collect();
        assert!(passguard_additions.is_empty());
    }

    #[test]
    fn undiscovered_deposit_of_same_type_does_not_block_the_new_discovered_one() {
        // spec §5.1: discovered=falseの鉱床は「既存の鉱床」として扱わない
        // (has_discovered_deposit相当の判定を再利用)。
        let mut states = target_states();
        let elfin = states.iter_mut().find(|s| s.id == StateId(16)).unwrap();
        elfin.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::Iron,
            base_output: 10.0,
            discovered: false,
            development_level: 1,
        });

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();
        assert!(report.added_deposits.contains(&(StateId(16), ResourceType::Iron)));
        let elfin = states.iter().find(|s| s.id == StateId(16)).unwrap();
        assert_eq!(
            elfin
                .resource_deposits
                .iter()
                .filter(|d| d.resource_type == ResourceType::Iron)
                .count(),
            2,
            "the pre-existing undiscovered deposit and the newly added discovered one coexist"
        );
    }

    #[test]
    fn other_resource_types_are_never_touched() {
        let mut states = target_states();
        let oceanic = states.iter_mut().find(|s| s.id == StateId(8)).unwrap();
        oceanic.resource_deposits.push(StateResourceDeposit {
            resource_type: ResourceType::MagicCrystal,
            base_output: 30.0,
            discovered: true,
            development_level: 1,
        });

        apply_minimum_supply_chain_migration(&mut states).unwrap();

        let oceanic = states.iter().find(|s| s.id == StateId(8)).unwrap();
        let crystal = deposit_of(oceanic, ResourceType::MagicCrystal).unwrap();
        assert_eq!(crystal.base_output, 30.0);
        assert_eq!(crystal.development_level, 1);
    }

    #[test]
    fn military_factory_already_at_level_1_is_left_unchanged() {
        let mut states = target_states();
        let oceanic = states.iter_mut().find(|s| s.id == StateId(8)).unwrap();
        oceanic
            .buildings
            .insert(BuildingType::MilitaryFactory, 1);

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert!(report.raised_buildings.is_empty());
        let oceanic = states.iter().find(|s| s.id == StateId(8)).unwrap();
        assert_eq!(oceanic.building_level(BuildingType::MilitaryFactory), 1);
    }

    #[test]
    fn military_factory_already_above_level_1_is_never_lowered() {
        let mut states = target_states();
        let oceanic = states.iter_mut().find(|s| s.id == StateId(8)).unwrap();
        oceanic
            .buildings
            .insert(BuildingType::MilitaryFactory, 4);

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert!(report.raised_buildings.is_empty());
        let oceanic = states.iter().find(|s| s.id == StateId(8)).unwrap();
        assert_eq!(oceanic.building_level(BuildingType::MilitaryFactory), 4);
    }

    #[test]
    fn other_buildings_on_state_8_are_never_touched() {
        let mut states = target_states();
        let oceanic = states.iter_mut().find(|s| s.id == StateId(8)).unwrap();
        oceanic.buildings.insert(BuildingType::Farm, 5);

        apply_minimum_supply_chain_migration(&mut states).unwrap();

        let oceanic = states.iter().find(|s| s.id == StateId(8)).unwrap();
        assert_eq!(oceanic.building_level(BuildingType::Farm), 5);
    }

    // ─── 冪等性(Test D相当) ───────────────────────────────────────

    #[test]
    fn applying_twice_is_idempotent() {
        let mut states = target_states();
        let first = apply_minimum_supply_chain_migration(&mut states).unwrap();
        let after_first = states.clone();
        let second = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert!(!first.is_empty());
        assert!(
            second.is_empty(),
            "second application must add nothing: {:?}",
            second
        );
        for (a, b) in after_first.iter().zip(states.iter()) {
            assert_eq!(a.resource_deposits.len(), b.resource_deposits.len());
            assert_eq!(a.buildings, b.buildings);
        }
    }

    #[test]
    fn applying_three_times_still_matches_the_single_application_result() {
        let mut states = target_states();
        apply_minimum_supply_chain_migration(&mut states).unwrap();
        let after_first = states.clone();
        apply_minimum_supply_chain_migration(&mut states).unwrap();
        apply_minimum_supply_chain_migration(&mut states).unwrap();

        for (a, b) in after_first.iter().zip(states.iter()) {
            assert_eq!(a.resource_deposits.len(), b.resource_deposits.len());
            assert_eq!(a.buildings, b.buildings);
        }
    }

    // ─── 原子性(Test G相当): 対象StateId欠落時は何も変更しない ────────

    #[test]
    fn missing_target_state_8_rejects_the_whole_migration_without_partial_changes() {
        let mut states: Vec<StateData> = vec![bare_state(13), bare_state(16), bare_state(21)]; // 8が欠落
        let before = states.clone();

        let result = apply_minimum_supply_chain_migration(&mut states);

        assert_eq!(result, Err(MissingTargetState(StateId(8))));
        for (a, b) in before.iter().zip(states.iter()) {
            assert_eq!(a.resource_deposits.len(), b.resource_deposits.len());
            assert!(b.resource_deposits.is_empty(), "no partial deposit must be added");
            assert_eq!(a.buildings, b.buildings);
        }
    }

    #[test]
    fn missing_target_state_13_rejects_the_whole_migration_without_partial_changes() {
        let mut states: Vec<StateData> = vec![bare_state(8), bare_state(16), bare_state(21)]; // 13が欠落
        let before_len: HashMap<StateId, usize> = states
            .iter()
            .map(|s| (s.id, s.resource_deposits.len()))
            .collect();

        let result = apply_minimum_supply_chain_migration(&mut states);

        assert_eq!(result, Err(MissingTargetState(StateId(13))));
        for s in &states {
            assert_eq!(s.resource_deposits.len(), before_len[&s.id]);
            assert!(s.buildings.is_empty());
        }
    }

    #[test]
    fn missing_state_check_runs_before_any_mutation_regardless_of_slice_order() {
        // 8を末尾に置いても(先頭にあるstateだけ処理してから欠落に気づく、という
        // 実装は許されない)、事前検証パスが先に全対象を確認するため無変更のまま拒否する。
        let mut states: Vec<StateData> = vec![bare_state(16), bare_state(21), bare_state(13)]; // 8が欠落、順序を変えても同じ結果
        let result = apply_minimum_supply_chain_migration(&mut states);
        assert_eq!(result, Err(MissingTargetState(StateId(8))));
        for s in &states {
            assert!(s.resource_deposits.is_empty());
        }
    }

    // ─── 対象外地図への配線(共有インフラ経由の既存テスト保護) ──────

    /// `src/save/apply.rs`・`src/app/loader.rs`という共有経路には、本タスクと無関係な
    /// 小規模テスト専用地図(State(0)/State(1)等)を使う既存テストが大量に存在する
    /// (`cargo test --lib save::`)。対象StateId(8/13/16/21)を1件も含まない地図に
    /// 対しては、エラーではなく「何もしない成功」を返す必要がある——さもなければ
    /// それら既存テストが本タスクと無関係な理由で軒並み失敗する(実際に発生し、
    /// この分岐を追加する直接のきっかけとなった)。
    #[test]
    fn a_map_containing_none_of_the_four_target_states_is_a_no_op_not_an_error() {
        let mut states: Vec<StateData> = vec![bare_state(0), bare_state(1), bare_state(2)];
        let before = states.clone();

        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();

        assert!(report.is_empty());
        for (a, b) in before.iter().zip(states.iter()) {
            assert_eq!(a.resource_deposits.len(), b.resource_deposits.len());
            assert_eq!(a.buildings, b.buildings);
        }
    }

    #[test]
    fn an_empty_state_list_is_also_a_no_op_not_an_error() {
        let mut states: Vec<StateData> = Vec::new();
        let report = apply_minimum_supply_chain_migration(&mut states).unwrap();
        assert!(report.is_empty());
    }

    #[test]
    fn at_least_one_target_state_present_but_not_all_still_triggers_atomic_rejection() {
        // 0件(対象外地図)と1〜3件(本番地図の破損)を明確に区別する: 1件でも対象
        // StateIdが存在すれば「本番地図に近いが壊れている」とみなし、原子的に拒否する。
        let mut states: Vec<StateData> = vec![bare_state(0), bare_state(1), bare_state(16)]; // 16だけ対象、他は無関係かつ8/13/21が欠落
        let result = apply_minimum_supply_chain_migration(&mut states);
        // placement一覧の宣言順(16,21,13,8)で最初に見つかった欠落StateIdが返る。
        assert_eq!(result, Err(MissingTargetState(StateId(21))));
        let state16 = states.iter().find(|s| s.id == StateId(16)).unwrap();
        assert!(state16.resource_deposits.is_empty(), "no partial application even for the one present target state");
    }

    // ─── 順序非依存性(HashMap由来の非決定順) ───────────────────────

    #[test]
    fn result_is_identical_regardless_of_input_slice_order() {
        let mut order_a = target_states();
        let mut order_b: Vec<StateData> = vec![
            bare_state(21),
            bare_state(8),
            bare_state(16),
            bare_state(13),
        ];

        apply_minimum_supply_chain_migration(&mut order_a).unwrap();
        apply_minimum_supply_chain_migration(&mut order_b).unwrap();

        for id in [8usize, 13, 16, 21] {
            let a = order_a.iter().find(|s| s.id == StateId(id)).unwrap();
            let b = order_b.iter().find(|s| s.id == StateId(id)).unwrap();
            assert_eq!(a.resource_deposits.len(), b.resource_deposits.len());
            assert_eq!(a.buildings, b.buildings);
        }
    }
}
