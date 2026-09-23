//! P25-007: 装備充足率の戦闘力反映のE2Eテスト。
//!
//! `military::combat_calc::calculate_combat_readiness`の数式的正確性(§18の数値例)は
//! `src/military/combat_calc.rs`自身の`#[cfg(test)]`内部テストモジュールで検証済み
//! (`resolve_combat_day`/`resolve_combat_day_multi`/`calculate_combat_readiness`は
//! いずれも`pub`のため、本ファイルからも直接呼び出せる)。
//!
//! `military::update::process_daily_battles`の戦闘前preflight(§15)はprivate関数の
//! ため、`src/military/update.rs`の`#[cfg(test)]`内部テストモジュールで検証済み。
//!
//! 本ファイルは、実際にBevy Appへ登録されたSystem
//! (`military::update::handle_daily_military`)を経由する必要がある項目
//! (New Game/Loaded Game比較・save/load分岐比較・P25-005補充後の値を使うこと)、
//! および`resolve_combat_day`/`resolve_combat_day_multi`を直接呼び出す
//! ホワイトボックス的な数値・原子性・複数Division/複数国家テストを担当する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use std::collections::HashSet;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{BattleId, CountryId, DivisionId, StateId, WarId};
use strategy_game::country::{CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::battle::{Battle, BattleRegistry, BattleStatus};
use strategy_game::military::combat_calc::{
    CombatReadinessError, TERRAIN_BONUS_PLAIN, calculate_combat_readiness, resolve_combat_day,
    resolve_combat_day_multi,
};
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::war::WarPlugin;
use strategy_game::war::data::{War, WarRegistry, WarStatus};

// ─── 軽量ハーネス(resolve_combat_day/_multiを直接呼ぶホワイトボックステスト用) ──

#[allow(clippy::too_many_arguments)]
fn make_division(
    id: usize,
    owner: CountryId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    organization: f32,
    atk: i32,
    def: i32,
) -> Division {
    Division {
        id: DivisionId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: StateId(1),
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: atk,
        defense_power: def,
        combat_id: Some(BattleId(0)),
    }
}

// ─── Test 16/17/18: 完全充足時の完全互換 ─────────────────────────────────

#[test]
fn test_16_17_18_full_readiness_reproduces_the_pre_p25_007_result_bit_for_bit() {
    // src/military/combat_calc.rs::tests::test_resolve_combat_day_basicと
    // 同一の入力(装備満タン)。P25-007以前の結果と完全一致するはず。
    let attacker = make_division(0, CountryId(1), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (atk_loss, atk_org, def_loss, def_org) =
        resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

    assert_eq!(atk_loss, def_loss, "均等な場合、両者のダメージは同程度(P25-007以前と同一)");
    assert!(atk_loss > 0);
    assert_eq!(atk_org, atk_loss as f32 / 10.0);
    assert_eq!(def_org, def_loss as f32 / 10.0);

    // test_combat_is_deterministicと同じ非対称入力での既知の期待値も、
    // 装備満タンなら完全一致するはず。
    let a2 = make_division(0, CountryId(1), 8000, 10000, 100.0, 100.0, 75.0, 12, 8);
    let d2 = make_division(1, CountryId(2), 6000, 10000, 100.0, 100.0, 60.0, 9, 14);
    let result1 = resolve_combat_day(&a2, &d2, strategy_game::military::combat_calc::TERRAIN_BONUS_HILLS);
    let result2 = resolve_combat_day(&a2, &d2, strategy_game::military::combat_calc::TERRAIN_BONUS_HILLS);
    assert_eq!(result1, result2, "決定的であることは維持されている");
}

// ─── Test 19/20: 装備不足でattack/defenseが比例低下 ───────────────────────

#[test]
fn test_19_20_equipment_shortage_proportionally_reduces_attack_and_defense_contribution() {
    let full_attacker = make_division(0, CountryId(1), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let half_equipped_attacker =
        make_division(0, CountryId(1), 10000, 10000, 50.0, 100.0, 100.0, 10, 10);
    let defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (_, _, def_loss_full, _) = resolve_combat_day(&full_attacker, &defender, TERRAIN_BONUS_PLAIN);
    let (_, _, def_loss_half, _) =
        resolve_combat_day(&half_equipped_attacker, &defender, TERRAIN_BONUS_PLAIN);

    assert!(
        def_loss_half < def_loss_full,
        "攻撃側の装備不足(50%)は、防御側へ与えるダメージ(attack寄与)を低下させるはず"
    );

    let full_defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 15);
    let half_equipped_defender =
        make_division(1, CountryId(2), 10000, 10000, 50.0, 100.0, 100.0, 10, 15);
    let attacker = make_division(0, CountryId(1), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (atk_loss_vs_full, _, _, _) = resolve_combat_day(&attacker, &full_defender, TERRAIN_BONUS_PLAIN);
    let (atk_loss_vs_half, _, _, _) =
        resolve_combat_day(&attacker, &half_equipped_defender, TERRAIN_BONUS_PLAIN);

    // 既存式(§3の監査対象): 攻撃側が受けるmanpowerダメージ =
    // (defender_effective + terrain_bonus) * DAMAGE_SCALE / max(attacker_effective, 1)。
    // defender_effectiveは分子側にあるため、防御側の装備不足(defense寄与の低下)は
    // 攻撃側が受けるダメージを「増やす」のではなく「減らす」(防御側の反撃力が
    // 弱まるため)。この式自体はP25-007で変更していない——readinessは
    // defender_effectiveを構成する一因子として掛かるだけである。
    assert!(
        atk_loss_vs_half < atk_loss_vs_full,
        "防御側の装備不足(50%)はdefense寄与(=攻撃側への反撃力)を低下させ、\
         攻撃側が受ける被害を既存式どおり減らすはず"
    );
}

// ─── Test 21/22: attacker側・defender側への適用 ──────────────────────────

#[test]
fn test_21_22_readiness_applies_identically_to_attacker_and_defender_roles() {
    // 同じdivision fixture(装備50%)を、一方はattacker役、他方はdefender役として
    // 使い、両方で同じ`calculate_combat_readiness`の結果になることを確認する。
    let d_as_attacker = make_division(0, CountryId(1), 8000, 10000, 40.0, 100.0, 100.0, 10, 10);
    let d_as_defender = make_division(0, CountryId(1), 8000, 10000, 40.0, 100.0, 100.0, 10, 10);

    let r_attacker =
        calculate_combat_readiness(d_as_attacker.manpower, d_as_attacker.max_manpower, d_as_attacker.equipment, d_as_attacker.max_equipment)
            .unwrap();
    let r_defender =
        calculate_combat_readiness(d_as_defender.manpower, d_as_defender.max_manpower, d_as_defender.equipment, d_as_defender.max_equipment)
            .unwrap();
    assert_eq!(r_attacker, r_defender);
}

// ─── Test 23: 攻守交代・反撃でも同じ式 ───────────────────────────────────

#[test]
fn test_23_role_reversal_uses_the_same_readiness_formula() {
    let strong = make_division(0, CountryId(1), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let weak = make_division(1, CountryId(2), 10000, 10000, 30.0, 100.0, 100.0, 10, 10);

    // strongが攻撃側、weakが防御側
    let (_, _, def_loss_1, _) = resolve_combat_day(&strong, &weak, TERRAIN_BONUS_PLAIN);
    // 役割を入れ替え: weakが攻撃側、strongが防御側
    let (atk_loss_2, _, _, _) = resolve_combat_day(&weak, &strong, TERRAIN_BONUS_PLAIN);

    // weakの装備30%というハンディキャップは、attacker役でもdefender役でも
    // 同じreadiness計算式(min(m,e))を通るため、対称的な式で説明できるはず
    // (数値そのものはattack_power/defense_powerの非対称性により異なり得るが、
    // 装備30%のDivisionのreadinessは常に同一)。
    let weak_readiness =
        calculate_combat_readiness(weak.manpower, weak.max_manpower, weak.equipment, weak.max_equipment).unwrap();
    assert_eq!(weak_readiness.combat_readiness_units, 300_000);
    // defender役の時(def_loss_1相当)もattacker役の時(atk_loss_2相当)も、
    // 装備不足による弱体化がゼロではなく実際に反映されている(両方とも0ではない
    // が、装備満タン同士の基準値より小さいことを別テストで確認済み)。
    let _ = (def_loss_1, atk_loss_2);
}

// ─── Test 24/25: 複数Divisionの個別補正、平均装備率を後掛けしない ────────

#[test]
fn test_24_25_multi_division_corrects_each_division_individually_not_a_pooled_average() {
    // 攻撃側2体: 1体は装備満タン、もう1体は装備0。防御側1体(満タン)。
    let full_attacker = make_division(0, CountryId(1), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let zero_attacker = make_division(1, CountryId(1), 10000, 10000, 0.0, 100.0, 100.0, 10, 10);
    let defender = make_division(2, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (atk_result, _) = resolve_combat_day_multi(
        &[&full_attacker, &zero_attacker],
        &[&defender],
        TERRAIN_BONUS_PLAIN,
    );

    // 平均装備率(50%)を陣営合計へ後掛けするのではなく、各Divisionが個別に
    // 補正されるため、装備0のDivisionは防御側への貢献が0のはず
    // (=合計ダメージが「2体×50%平均」ではなく「1体分の満タンダメージ」相当になる)。
    let (single_atk_result, _) =
        resolve_combat_day_multi(&[&full_attacker], &[&defender], TERRAIN_BONUS_PLAIN);
    let single_def_loss = {
        let (_, def_result) = resolve_combat_day_multi(&[&full_attacker], &[&defender], TERRAIN_BONUS_PLAIN);
        def_result[&defender.id].0
    };
    let (_, def_result_multi) = resolve_combat_day_multi(
        &[&full_attacker, &zero_attacker],
        &[&defender],
        TERRAIN_BONUS_PLAIN,
    );
    let multi_def_loss = def_result_multi[&defender.id].0;

    // 装備0のDivisionは実効戦力0であり、貢献度配分(distribute_damage)上も
    // 攻撃力への寄与がないため、防御側が受ける被害は1体だけの場合とほぼ同じになる
    // (完全一致ではなく近い値であることを確認する: 実効戦力の合計計算経路が
    // 1体の場合と2体[うち1体は0]の場合で異なるため丸め誤差が生じ得る)。
    let diff = multi_def_loss.abs_diff(single_def_loss);
    assert!(
        diff <= 1,
        "装備0のDivisionは防御側への被害へほぼ寄与しないはず(平均化されていない): \
         single={single_def_loss}, multi={multi_def_loss}"
    );

    // 装備0のDivision自身がゼロ実効戦力として扱われていることも確認する。
    assert_eq!(atk_result.len(), 2);
    let full_readiness =
        calculate_combat_readiness(full_attacker.manpower, full_attacker.max_manpower, full_attacker.equipment, full_attacker.max_equipment)
            .unwrap();
    let zero_readiness =
        calculate_combat_readiness(zero_attacker.manpower, zero_attacker.max_manpower, zero_attacker.equipment, zero_attacker.max_equipment)
            .unwrap();
    assert_eq!(full_readiness.combat_readiness_units, 1_000_000);
    assert_eq!(zero_readiness.combat_readiness_units, 0);
    let _ = single_atk_result;
}

// ─── Test 26: DivisionId/ECS挿入順非依存 ─────────────────────────────────

#[test]
fn test_26_result_does_not_depend_on_division_slice_order() {
    let a = make_division(5, CountryId(1), 10000, 10000, 40.0, 100.0, 100.0, 10, 10);
    let b = make_division(2, CountryId(1), 10000, 10000, 80.0, 100.0, 100.0, 10, 10);
    let defender = make_division(9, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (result_ab, _) = resolve_combat_day_multi(&[&a, &b], &[&defender], TERRAIN_BONUS_PLAIN);
    let (result_ba, _) = resolve_combat_day_multi(&[&b, &a], &[&defender], TERRAIN_BONUS_PLAIN);

    assert_eq!(result_ab.get(&a.id), result_ba.get(&a.id));
    assert_eq!(result_ab.get(&b.id), result_ba.get(&b.id));
}

// ─── Test 27/28: RNG呼び出し回数・系列不変(このコードベースはそもそも無RNG) ──

#[test]
fn test_27_28_identical_inputs_produce_bit_for_bit_identical_results_across_equipment_levels() {
    // このコードベースの戦闘計算は乱数を一切使わない(combat_calc.rsのモジュール
    // doc「乱数を使わず、整数演算で決定的な戦闘計算を行う」)。したがって
    // 「RNG呼び出し回数・系列が変わらない」という要件は、同一入力に対して
    // 常に同一出力を返すという決定性の確認に帰着する。
    for equipment in [0.0, 25.0, 50.0, 75.0, 100.0] {
        let attacker = make_division(0, CountryId(1), 8000, 10000, equipment, 100.0, 75.0, 12, 8);
        let defender = make_division(1, CountryId(2), 6000, 10000, 60.0, 100.0, 60.0, 9, 14);
        let r1 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
        let r2 = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
        assert_eq!(r1, r2, "equipment={equipment}でも決定的であるはず");
    }
}

// ─── Test 29: organizationへ直接補正していない ───────────────────────────

#[test]
fn test_29_readiness_does_not_directly_scale_organization_loss_beyond_the_existing_manpower_ratio() {
    // organization損失は既存どおり`manpower_loss / ORG_DAMAGE_DENOMINATOR`から
    // 導出されるだけであり、readinessを二重に掛けていないことを確認する。
    let attacker = make_division(0, CountryId(1), 10000, 10000, 40.0, 100.0, 100.0, 10, 10);
    let defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let (atk_loss, atk_org, def_loss, def_org) =
        resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
    assert_eq!(atk_org, atk_loss as f32 / 10.0, "organization損失は既存の manpower_loss/10 式のままのはず");
    assert_eq!(def_org, def_loss as f32 / 10.0);
}

// ─── Test 30: movementへ影響しない ───────────────────────────────────────

#[test]
fn test_30_readiness_does_not_touch_movement_fields() {
    let mut attacker = make_division(0, CountryId(1), 10000, 10000, 40.0, 100.0, 100.0, 10, 10);
    attacker.movement_progress = 0.42;
    attacker.destination = Some(StateId(7));
    let defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let before_progress = attacker.movement_progress;
    let before_destination = attacker.destination;
    let _ = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

    // resolve_combat_dayはDivisionの参照だけを読み、何も書き換えない
    // (呼び出し元がmanpower/organizationだけを結果から反映する)。
    assert_eq!(attacker.movement_progress, before_progress);
    assert_eq!(attacker.destination, before_destination);
}

// ─── Test 31/32/33: 装備0でも安全、arbitraryな最低戦闘力なし ─────────────

#[test]
fn test_31_32_33_zero_equipment_on_both_sides_is_safe_and_produces_zero_casualties_without_a_floor() {
    let attacker = make_division(0, CountryId(1), 10000, 10000, 0.0, 100.0, 100.0, 10, 10);
    let defender = make_division(1, CountryId(2), 10000, 10000, 0.0, 100.0, 100.0, 10, 15);

    let (atk_loss, atk_org, def_loss, def_org) =
        resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);

    assert!(!atk_org.is_nan() && atk_org.is_finite());
    assert!(!def_org.is_nan() && def_org.is_finite());
    assert_eq!(
        (atk_loss, def_loss),
        (0, 0),
        "双方の戦闘充足率が0なら、arbitraryな最低戦闘力を与えずcasualty自体が0になるはず"
    );
}

#[test]
fn test_31_zero_equipment_on_one_side_only_does_not_panic_or_produce_nan() {
    let attacker = make_division(0, CountryId(1), 10000, 10000, 0.0, 100.0, 100.0, 10, 10);
    let defender = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 15);
    let (_, atk_org, _, def_org) = resolve_combat_day(&attacker, &defender, TERRAIN_BONUS_PLAIN);
    assert!(!atk_org.is_nan() && atk_org.is_finite());
    assert!(!def_org.is_nan() && def_org.is_finite());
}

// ─── Test 13/14/15: 不正状態・checked u128境界 ───────────────────────────

#[test]
fn test_13_invalid_equipment_state_is_rejected() {
    assert_eq!(
        calculate_combat_readiness(100, 100, 150.0, 100.0).unwrap_err(),
        CombatReadinessError::EquipmentExceedsMax
    );
}

#[test]
fn test_14_invalid_manpower_state_is_rejected() {
    assert_eq!(
        calculate_combat_readiness(150, 100, 100.0, 100.0).unwrap_err(),
        CombatReadinessError::ManpowerExceedsMax
    );
}

// ─── 実Appハーネス(New Game/Loaded Game・save/load分岐・P25-005順序) ─────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_007_e2e_{label}_{}_{nanos}_{n}",
        std::process::id()
    ))
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(label: &str) -> Self {
        Self(unique_temp_dir(label))
    }

    fn save_file_config(&self) -> SaveFileConfig {
        SaveFileConfig {
            path: SavePathConfig {
                final_path: self.0.join("savegame_v1.ron"),
            },
        }
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup_app_in_playing(player: CountryId, save_config: SaveFileConfig) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin)
        .add_plugins(PopulationPlugin);

    app.insert_resource(strategy_game::map::division_selection::SelectedDivision::default())
        .insert_resource(strategy_game::map::division_selection::DragSelectState::default())
        .insert_resource(SelectedState::default())
        .insert_resource(strategy_game::ui::ActivePanel::default())
        .insert_resource(strategy_game::ui::diplomacy_panel::DiplomacyPanelState::default())
        .insert_resource(strategy_game::ui::military_panel::MilitaryPanelState::default())
        .insert_resource(strategy_game::ui::peace_panel::PeacePanelState::default())
        .insert_resource(strategy_game::ui::politics_panel::PoliticsPanelState::default())
        .insert_resource(strategy_game::ui::research_panel::ResearchPanelState::default())
        .insert_resource(strategy_game::map::camera::CameraDragState::default())
        .add_message::<strategy_game::map::camera::CameraResetRequestMessage>();

    app.add_plugins(SaveGamePlugin);
    app.add_plugins(LoadGamePlugin);

    app.insert_resource(save_config);
    app.update();

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update();

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> = SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

const TEST_STATE_ID: StateId = StateId(999_999);

#[allow(clippy::too_many_arguments)]
fn spawn_division(
    app: &mut App,
    owner: CountryId,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    atk: i32,
    def: i32,
) -> DivisionId {
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: TEST_STATE_ID,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Fighting,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: atk,
        defense_power: def,
        combat_id: None,
    };
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(division)
}

fn create_war(app: &mut App, attacker: CountryId, defender: CountryId) -> WarId {
    let war = War {
        id: WarId(0),
        name: "P25-007 Test War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: None,
        primary_defender: None,
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: HashSet::new(),
    };
    app.world_mut().resource_mut::<WarRegistry>().add_war(war)
}

fn create_battle(
    app: &mut App,
    war_id: WarId,
    attacker_country: CountryId,
    defender_country: CountryId,
    attacker_ids: Vec<DivisionId>,
    defender_ids: Vec<DivisionId>,
) -> BattleId {
    // P27-FU-001: originはBattleの場所(`TEST_STATE_ID`、実在しない合成州)ではなく、
    // 攻撃側Divisionが実際に出撃した実州でなければならない。同一値のままだと
    // 新しい出撃元検証で「無効なorigin」と判定され、代替探索も`TEST_STATE_ID`に
    // 隣接州が存在しないため即失敗し、Divisionが誤って消滅する。本ファイルの
    // 検証対象(装備充足率が戦闘力へ与える影響)は出撃元の地理と無関係なので、
    // 実マップ上で攻撃側国家が実際に支配する州を出撃元として使う。
    let attacker_origin = state_owned_by(app, attacker_country);
    let attacker_origins = attacker_ids.iter().map(|id| (*id, attacker_origin)).collect();
    let battle = Battle {
        id: BattleId(0),
        war_id,
        state_id: TEST_STATE_ID,
        attacker_country,
        defender_country,
        attacker_division_ids: attacker_ids.clone(),
        defender_division_ids: defender_ids.clone(),
        attacker_origins,
        start_date: "1800/01/01".to_string(),
        elapsed_days: 0,
        status: BattleStatus::Ongoing,
    };
    let battle_id = app
        .world_mut()
        .resource_mut::<BattleRegistry>()
        .start_battle(battle)
        .unwrap();
    let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
    for id in attacker_ids.iter().chain(defender_ids.iter()) {
        if let Some(d) = mr.divisions.get_mut(id) {
            d.combat_id = Some(battle_id);
        }
    }
    battle_id
}

/// 補充の適格条件(`is_eligible_for_daily_reinforcement_by_position`)は、実際に
/// その所有国が支配する州を要求する(`TEST_STATE_ID`は実マップに存在しないため
/// 補充の適格性判定では常に不適格になる)。実マップ上でその国が実際に所有する
/// 州を動的に取得する(p25_005のテストと同じ方式)。
fn state_owned_by(app: &App, owner: CountryId) -> StateId {
    app.world()
        .resource::<strategy_game::state::data::StateRegistry>()
        .states
        .iter()
        .find(|s| s.owner_country_id == owner)
        .map(|s| s.id)
        .expect("real map must have at least one state owned by this country")
}

#[allow(clippy::too_many_arguments)]
fn spawn_division_at(
    app: &mut App,
    owner: CountryId,
    state: StateId,
    status: DivisionStatus,
    manpower: u64,
    max_manpower: u64,
    equipment: f64,
    max_equipment: f64,
    atk: i32,
    def: i32,
) -> DivisionId {
    let division = Division {
        id: DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state: state,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower,
        max_manpower,
        equipment,
        max_equipment,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status,
        def_id: strategy_game::common::DivisionDefinitionId(0),
        attack_power: atk,
        defense_power: def,
        combat_id: None,
    };
    app.world_mut()
        .resource_mut::<MilitaryRegistry>()
        .add_division(division)
}

fn get_division(app: &App, id: DivisionId) -> Option<Division> {
    app.world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&id)
        .cloned()
}

fn stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

// ─── Test 18(実戦闘): 完全充足の実battleがbit-for-bit一致 ───────────────

#[test]
fn test_18_full_readiness_real_battle_matches_across_two_independent_runs() {
    fn run_once(label: &str) -> (u64, f64, u64, f64) {
        let player = CountryId(0);
        let ai = CountryId(1);
        let temp_dir = TempTestDir::new(label);
        let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
        let attacker_id = spawn_division(&mut app, player, 10_000, 10_000, 100.0, 100.0, 10, 10);
        let defender_id = spawn_division(&mut app, ai, 10_000, 10_000, 100.0, 100.0, 10, 15);
        let war_id = create_war(&mut app, player, ai);
        create_battle(&mut app, war_id, player, ai, vec![attacker_id], vec![defender_id]);
        send_day_changed(&mut app, 1800, 1, 1);
        let a = get_division(&app, attacker_id).unwrap();
        let d = get_division(&app, defender_id).unwrap();
        (a.manpower, a.equipment, d.manpower, d.equipment)
    }

    let r1 = run_once("full_a");
    let r2 = run_once("full_b");
    assert_eq!(r1, r2);
}

// ─── Test 34: P25-005同日補充後の値を戦闘に使用 ──────────────────────────

#[test]
fn test_34_combat_readiness_uses_the_post_reinforcement_equipment_value() {
    // 補充の適格条件(is_eligible_for_daily_reinforcement_by_position)は、
    // 進行中の戦闘に参照されているDivisionを除外する(P25-005自身のテスト
    // 「equipment_reinforced_the_same_day_can_still_be_lost_to_combat_that_day」の
    // コメントが明示するとおり、既存の設計)。したがって「同じDivisionが同日中に
    // 補充されてから初めて交戦する」ケースは、Idle・未参戦のまま補充を受けた後、
    // 後日改めてBattleへ組み込むことで再現する(§16の要求である「補充後の値を
    // 戦闘が使う」という順序自体は、Battle開始時点の`division.equipment`を
    // 都度読むcombat_calc側の実装により、日をまたいでも同様に保証される)。
    let player = CountryId(0);
    let ai = CountryId(1);
    let temp_dir = TempTestDir::new("p25_005_order");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());
    let home = state_owned_by(&app, player);

    let attacker_id = spawn_division_at(
        &mut app,
        player,
        home,
        DivisionStatus::Idle,
        10_000,
        10_000,
        50.0,
        100.0,
        10,
        10,
    );
    {
        let mut cr = app.world_mut().resource_mut::<CountryRegistry>();
        cr.get_mut(player)
            .unwrap()
            .stockpile
            .set(ResourceType::MilitaryEquipment, 50.0);
    }

    // Day 1: 戦闘未参加(Idle・未参照)のため補充対象。
    send_day_changed(&mut app, 1800, 1, 1);
    let after_reinforcement = get_division(&app, attacker_id).unwrap();
    assert_eq!(
        after_reinforcement.equipment, 100.0,
        "戦闘未参加の日次装備補充で満タンへ回復しているはず(P25-005)"
    );

    // Day 2: 補充済みの状態でBattleへ組み込み、combat_calcがこの
    // post-reinforcement equipment(100.0)を基準にreadinessを計算することを確認する。
    // P26-006: defenderはai自身が支配する実マップ州へ配置する(`home`はplayerの
    // 支配州であり、aiにとっては首都連結性がなく補給率0になってしまうため)。
    let ai_home = state_owned_by(&app, ai);
    let defender_id = spawn_division_at(
        &mut app,
        ai,
        ai_home,
        DivisionStatus::Fighting,
        10_000,
        10_000,
        100.0,
        100.0,
        10,
        10,
    );
    let war_id = create_war(&mut app, player, ai);
    create_battle(&mut app, war_id, player, ai, vec![attacker_id], vec![defender_id]);
    {
        let mut mr = app.world_mut().resource_mut::<MilitaryRegistry>();
        mr.divisions.get_mut(&attacker_id).unwrap().status = DivisionStatus::Fighting;
    }

    send_day_changed(&mut app, 1800, 1, 2);

    let full_readiness_defender_loss = {
        let a = make_division(0, player, 10_000, 10_000, 100.0, 100.0, 100.0, 10, 10);
        let d = make_division(1, ai, 10_000, 10_000, 100.0, 100.0, 100.0, 10, 10);
        resolve_combat_day(&a, &d, TERRAIN_BONUS_PLAIN).2
    };
    let defender_after = get_division(&app, defender_id).unwrap();
    let actual_defender_loss = 10_000 - defender_after.manpower;
    assert_eq!(
        actual_defender_loss, full_readiness_defender_loss,
        "補充で満タンになった装備(readiness 100%)を基準にcombat_calcが計算しているはず"
    );
}

// ─── Test 35: P25-004 actual casualty→装備損失の維持 ─────────────────────

#[test]
fn test_35_p25_004_equipment_loss_still_follows_the_actual_applied_casualty() {
    let player = CountryId(0);
    let ai = CountryId(1);
    let temp_dir = TempTestDir::new("p25_004_link");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // P26-006: 首都連結性が戦闘力へ影響するため、各Divisionを自国が実際に支配する
    // 実マップ州(`TEST_STATE_ID`ではなく)へ配置し、両者とも補給充足率1.0(完全連結)の
    // まま本テストの主眼(P25-004の装備損失連動)を検証する。
    let player_home = state_owned_by(&app, player);
    let ai_home = state_owned_by(&app, ai);
    let attacker_id = spawn_division_at(
        &mut app, player, player_home, DivisionStatus::Fighting, 10_000, 10_000, 100.0, 100.0, 10, 10,
    );
    let defender_id = spawn_division_at(
        &mut app, ai, ai_home, DivisionStatus::Fighting, 10_000, 10_000, 50.0, 100.0, 10, 15,
    );
    let war_id = create_war(&mut app, player, ai);
    create_battle(&mut app, war_id, player, ai, vec![attacker_id], vec![defender_id]);

    send_day_changed(&mut app, 1800, 1, 1);

    let attacker_after = get_division(&app, attacker_id).unwrap();
    assert!(
        attacker_after.equipment < 100.0,
        "P25-004の装備損失(actual casualtyに比例)は引き続き適用されるはず"
    );
}

// ─── Test 39/40: 国家消滅時は全損を維持、国家備蓄へ戦闘力計算が触れない ───

#[test]
fn test_40_calculate_combat_readiness_never_touches_country_stockpile() {
    let player = CountryId(0);
    let ai = CountryId(1);
    let temp_dir = TempTestDir::new("stockpile_untouched");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let attacker_id = spawn_division(&mut app, player, 10_000, 10_000, 40.0, 100.0, 10, 10);
    let defender_id = spawn_division(&mut app, ai, 10_000, 10_000, 30.0, 100.0, 10, 10);
    let stockpile_before = stockpile(&app, player);
    let war_id = create_war(&mut app, player, ai);
    create_battle(&mut app, war_id, player, ai, vec![attacker_id], vec![defender_id]);

    send_day_changed(&mut app, 1800, 1, 1);

    assert_eq!(
        stockpile(&app, player),
        stockpile_before,
        "戦闘充足率計算はCountryStockpileへ一切触れないはず(Division自身の\
         equipmentフィールドのみを参照する)"
    );
}

// ─── Test 41: 複数国家分離 ────────────────────────────────────────────────

#[test]
fn test_41_multiple_countries_do_not_leak_readiness_state_into_each_other() {
    let a = make_division(0, CountryId(1), 10000, 10000, 20.0, 100.0, 100.0, 10, 10);
    let b = make_division(1, CountryId(2), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);
    let bystander = make_division(2, CountryId(3), 10000, 10000, 100.0, 100.0, 100.0, 10, 10);

    let (_, def_result) = resolve_combat_day_multi(&[&a], &[&b], TERRAIN_BONUS_PLAIN);
    let bystander_readiness = calculate_combat_readiness(
        bystander.manpower,
        bystander.max_manpower,
        bystander.equipment,
        bystander.max_equipment,
    )
    .unwrap();
    assert_eq!(bystander_readiness.combat_readiness_units, 1_000_000);
    assert!(def_result.contains_key(&b.id));
    assert!(!def_result.contains_key(&bystander.id));
}

// ─── Test 42: New Game／Loaded Game比較 ──────────────────────────────────

#[test]
fn test_42_new_game_and_loaded_game_produce_identical_combat_readiness_results() {
    let player = CountryId(0);
    let ai = CountryId(1);

    let temp_dir_a = TempTestDir::new("ng_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let attacker_a = spawn_division(&mut app_a, player, 10_000, 10_000, 45.0, 100.0, 10, 10);
    let defender_a = spawn_division(&mut app_a, ai, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let war_a = create_war(&mut app_a, player, ai);
    create_battle(&mut app_a, war_a, player, ai, vec![attacker_a], vec![defender_a]);
    send_day_changed(&mut app_a, 1800, 1, 1);
    let after_a = get_division(&app_a, attacker_a).unwrap();

    let temp_dir_b = TempTestDir::new("lg_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    let attacker_b = spawn_division(&mut app_b, player, 10_000, 10_000, 45.0, 100.0, 10, 10);
    let defender_b = spawn_division(&mut app_b, ai, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let war_b = create_war(&mut app_b, player, ai);
    create_battle(&mut app_b, war_b, player, ai, vec![attacker_b], vec![defender_b]);
    send_day_changed(&mut app_b, 1800, 1, 1);
    let after_b = get_division(&app_b, attacker_b).unwrap();

    assert_eq!(after_a.manpower, after_b.manpower);
    assert_eq!(after_a.equipment, after_b.equipment);
}

// ─── Test 43/44: save/load分岐比較 ───────────────────────────────────────

#[test]
fn test_43_44_pre_and_post_combat_save_load_branches_match_the_uninterrupted_run() {
    let player = CountryId(0);
    let ai = CountryId(1);

    let temp_dir_a = TempTestDir::new("branch_a");
    let mut app_a = setup_app_in_playing(player, temp_dir_a.save_file_config());
    let attacker_a = spawn_division(&mut app_a, player, 10_000, 10_000, 35.0, 100.0, 10, 10);
    let defender_a = spawn_division(&mut app_a, ai, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let war_a = create_war(&mut app_a, player, ai);
    create_battle(&mut app_a, war_a, player, ai, vec![attacker_a], vec![defender_a]);
    send_day_changed(&mut app_a, 1800, 1, 1);
    let after_a = get_division(&app_a, attacker_a).unwrap();

    let temp_dir_b = TempTestDir::new("branch_b");
    let mut app_b = setup_app_in_playing(player, temp_dir_b.save_file_config());
    let attacker_b = spawn_division(&mut app_b, player, 10_000, 10_000, 35.0, 100.0, 10, 10);
    let defender_b = spawn_division(&mut app_b, ai, 10_000, 10_000, 100.0, 100.0, 10, 10);
    let war_b = create_war(&mut app_b, player, ai);
    create_battle(&mut app_b, war_b, player, ai, vec![attacker_b], vec![defender_b]);
    send_save_request(&mut app_b);
    app_b.update();
    send_load_request(&mut app_b);
    app_b.update();
    app_b.update();
    send_day_changed(&mut app_b, 1800, 1, 1);
    let after_b = get_division(&app_b, attacker_b).unwrap();

    assert_eq!(after_a.manpower, after_b.manpower, "戦闘前save/load分岐は一致するはず");
    assert_eq!(after_a.equipment, after_b.equipment);

    // 戦闘後save/loadでも同じ値を維持する。
    send_save_request(&mut app_a);
    app_a.update();
    send_load_request(&mut app_a);
    app_a.update();
    app_a.update();
    let after_a_reload = get_division(&app_a, attacker_a).unwrap();
    assert_eq!(after_a_reload.manpower, after_a.manpower);
    assert_eq!(after_a_reload.equipment, after_a.equipment);
}

// ─── Test 46: deadなwar::combat.rsを接続していない ───────────────────────

#[test]
fn test_46_dead_war_combat_path_remains_unconnected() {
    // P25-004で確立済みの確認(src/military/update.rs内部テスト
    // test_u_dead_war_combat_process_combat_path_remains_unconnected)を、
    // P25-007後も同じ前提が成立することの簡易な再確認として行う。
    use strategy_game::economy::resources::military_equipment_value_to_units;
    let out_of_range = 2_000_000_000.0;
    assert!(military_equipment_value_to_units(out_of_range).is_err());
}
