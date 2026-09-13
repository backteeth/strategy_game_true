//! P25-003: 募兵時の軍事装備支給接続のE2Eテスト。
//!
//! Test A〜K・M・R・S・U・Vは`military::recruitment::{request_recruitment,
//! process_recruitment, cancel_recruitment}`を直接呼び出す軽量ハーネス
//! (`CountryRegistry`/`MilitaryRegistry`を手動構築、実Appなし)で検証する。
//! Test L・N・O・P・Q・Tは、実際のSystem実行順序・Save/Load・New Game初期化・
//! AI月次判断を経由する必要があるため、実App(`AppPlugin`ほか実プラグイン一式)を
//! 使用する。

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use strategy_game::app::AppPlugin;
use strategy_game::app::game_state::GameState;
use strategy_game::app::time::DayChangedMessage;
use strategy_game::building::BuildingPlugin;
use strategy_game::common::{CountryId, DivisionDefinitionId, StateId};
use strategy_game::country::{CountryData, CountryPlugin, CountryRegistry, PlayerCountry};
use strategy_game::diplomacy::DiplomacyPlugin;
use strategy_game::economy::EconomyPlugin;
use strategy_game::economy::resources::ResourceType;
use strategy_game::military::MilitaryPlugin;
use strategy_game::military::data::{
    DivisionDefinition, DivisionSize, DivisionType, MilitaryRegistry,
};
use strategy_game::military::recruitment::{process_recruitment, request_recruitment};
use strategy_game::politics::PoliticsPlugin;
use strategy_game::population::PopulationPlugin;
use strategy_game::research::ResearchPlugin;
use strategy_game::save::runtime::{LoadRequestMessage, SaveFileConfig};
use strategy_game::save::write::SavePathConfig;
use strategy_game::save::{LoadGamePlugin, SaveGamePlugin, SaveRequestMessage};
use strategy_game::state::SelectedState;
use strategy_game::state::StatePlugin;
use strategy_game::war::WarPlugin;

// ─── 軽量ハーネス(Test A〜K・M・R・S・U・V用) ──────────────────────────

fn registry_with_required_equipment(required_equipment: f64) -> MilitaryRegistry {
    let mut registry = MilitaryRegistry::default();
    registry.definitions.insert(
        DivisionDefinitionId(1),
        DivisionDefinition {
            id: DivisionDefinitionId(1),
            name: "Test Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment,
            recruitment_days: 30,
            movement_speed: 4.0,
            attack: 10.0,
            defense: 15.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 100.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        },
    );
    registry
}

fn setup_registry() -> MilitaryRegistry {
    registry_with_required_equipment(100.0)
}

fn fresh_country(id: usize, available_manpower: u64) -> CountryData {
    CountryData {
        id: CountryId(id),
        available_manpower,
        ..CountryData::default()
    }
}

fn run_days(
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    days: u32,
) {
    for _ in 0..days {
        process_recruitment(country_registry, military_registry);
    }
}

// ─── Test A: 正常募兵 ────────────────────────────────────────────────────

#[test]
fn test_a_normal_recruitment_moves_manpower_and_equipment_atomically() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country.treasury = 500.0;
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    country_registry.countries.push(country);

    run_days(&mut country_registry, &mut military_registry, 30);

    let c = &country_registry.countries[0];
    assert_eq!(c.available_manpower, 10_000);
    assert_eq!(c.mobilized_manpower, 10_000);
    assert_eq!(c.stockpile.get(ResourceType::MilitaryEquipment), 0.0);
    assert_eq!(
        c.treasury, 500.0,
        "required_equipment must not affect treasury"
    );
    assert_eq!(c.recruitment_queue.len(), 0);
    assert_eq!(military_registry.divisions.len(), 1);
    let division = military_registry.divisions.values().next().unwrap();
    assert_eq!(division.equipment, 100.0);
    assert_eq!(division.max_equipment, 100.0);
}

// ─── Test B: 装備ちょうど ────────────────────────────────────────────────

#[test]
fn test_b_stockpile_exactly_matching_requirement_succeeds_and_empties_the_stockpile() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    country_registry.countries.push(country);
    run_days(&mut country_registry, &mut military_registry, 30);

    assert_eq!(military_registry.divisions.len(), 1);
    assert_eq!(
        country_registry.countries[0]
            .stockpile
            .get(ResourceType::MilitaryEquipment),
        0.0
    );
}

// ─── Test C: 装備不足(P27-001: 完成を遅延させず0装備で完成する) ─────────
//
// P27-001以前は、備蓄不足(0.0 < 必要100.0)の場合、`try_spawn_division`が
// 常に必要量の全額を要求(all-or-nothing)していたため訓練完了後もqueueに
// 無期限滞留していた(旧テスト名: `_delays_completion_...`)。P27-001は
// 候補C(P27-001-PRE-001)を採用し、利用可能な範囲(この場合は0)だけを支給して
// 即座に完成させる——これは意図的な契約の反転であり、以下は新しい正規挙動を
// 固定する。

#[test]
fn test_c_zero_equipment_stockpile_completes_immediately_with_a_zero_equipped_division() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country.treasury = 500.0;
    // stockpile[MilitaryEquipment]は既定の0のまま(必要100.0に対して不足)。

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    let manpower_after_request = country.available_manpower;
    let mobilized_after_request = country.mobilized_manpower;
    country_registry.countries.push(country);

    run_days(&mut country_registry, &mut military_registry, 30);

    let c = &country_registry.countries[0];
    assert_eq!(
        military_registry.divisions.len(),
        1,
        "P27-001: equipment shortage must no longer delay completion"
    );
    let division = military_registry.divisions.values().next().unwrap();
    assert_eq!(division.equipment, 0.0, "issued equipment must be exactly what was available (0)");
    assert_eq!(
        division.max_equipment, 100.0,
        "max_equipment must stay at the full required_equipment, not the issued amount \
         (otherwise later daily equipment reinforcement would never top this division up)"
    );
    assert_eq!(c.available_manpower, manpower_after_request, "manpower accounting is unaffected");
    assert_eq!(c.mobilized_manpower, mobilized_after_request, "manpower accounting is unaffected");
    assert_eq!(c.stockpile.get(ResourceType::MilitaryEquipment), 0.0, "nothing was available to issue");
    assert_eq!(c.treasury, 500.0);
    assert_eq!(c.recruitment_queue.len(), 0, "the item must be removed from the queue on completion");
}

// ─── Test D: manpower不足 ────────────────────────────────────────────────

#[test]
fn test_d_insufficient_manpower_is_rejected_at_request_time_even_with_sufficient_equipment() {
    let military_registry = setup_registry();
    let mut country = fresh_country(1, 5_000); // 必要10,000に対して不足
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    let result = request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    );
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
    assert_eq!(
        country.stockpile.get(ResourceType::MilitaryEquipment),
        100.0,
        "equipment stockpile must be untouched when manpower alone blocks the request"
    );
}

// ─── Test E: 両方不足 ────────────────────────────────────────────────────

#[test]
fn test_e_both_manpower_and_equipment_insufficient_leaves_everything_unchanged() {
    let military_registry = setup_registry();
    let mut country = fresh_country(1, 5_000);
    country.treasury = 500.0;

    let result = request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    );
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
    assert_eq!(country.available_manpower, 5_000);
    assert_eq!(country.mobilized_manpower, 0);
    assert_eq!(country.stockpile.get(ResourceType::MilitaryEquipment), 0.0);
    assert_eq!(country.treasury, 500.0);
}

// ─── Test F: required_equipment=0 ───────────────────────────────────────

#[test]
fn test_f_zero_required_equipment_recruits_successfully_with_an_empty_stockpile() {
    let mut military_registry = registry_with_required_equipment(0.0);
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country.treasury = 500.0;

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    country_registry.countries.push(country);
    run_days(&mut country_registry, &mut military_registry, 30);

    assert_eq!(military_registry.divisions.len(), 1);
    let division = military_registry.divisions.values().next().unwrap();
    assert_eq!(division.equipment, 0.0);
    assert_eq!(division.max_equipment, 0.0);
    let c = &country_registry.countries[0];
    assert_eq!(c.stockpile.get(ResourceType::MilitaryEquipment), 0.0);
    assert_eq!(c.treasury, 500.0);
}

// ─── Test G: 小数装備 ────────────────────────────────────────────────────

#[test]
fn test_g_fractional_required_equipment_is_issued_exactly_without_drift() {
    let mut military_registry = registry_with_required_equipment(3.7);
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 200_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    for _ in 0..5 {
        request_recruitment(
            &mut country,
            &military_registry,
            DivisionDefinitionId(1),
            StateId(1),
        )
        .unwrap();
    }
    country_registry.countries.push(country);
    run_days(&mut country_registry, &mut military_registry, 30);

    assert_eq!(military_registry.divisions.len(), 5);
    for division in military_registry.divisions.values() {
        assert_eq!(division.equipment, 3.7);
        assert_eq!(division.max_equipment, 3.7);
    }
    // 保存則: 初期stockpile == 最終stockpile + 支給合計(ドリフトがないことの確認)。
    let final_stockpile = country_registry.countries[0]
        .stockpile
        .get(ResourceType::MilitaryEquipment);
    let total_issued: f64 = military_registry
        .divisions
        .values()
        .map(|d| d.equipment)
        .sum();
    assert_eq!(100.0, final_stockpile + total_issued);
}

// ─── Test H: 格子外値 ────────────────────────────────────────────────────

#[test]
fn test_h_off_grid_required_equipment_uses_one_consistent_canonical_value() {
    use strategy_game::economy::resources::military_equipment_value_to_units;

    let raw_required = 3.123_456_7; // 小数点以下7桁、scale(6桁)の格子外
    let mut military_registry = registry_with_required_equipment(raw_required);
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    country_registry.countries.push(country);
    run_days(&mut country_registry, &mut military_registry, 30);

    let c = &country_registry.countries[0];
    let division = military_registry.divisions.values().next().unwrap();

    let stockpile_before_units = military_equipment_value_to_units(100.0).unwrap();
    let stockpile_after_units =
        military_equipment_value_to_units(c.stockpile.get(ResourceType::MilitaryEquipment))
            .unwrap();
    let division_equipment_units = military_equipment_value_to_units(division.equipment).unwrap();
    let division_max_units = military_equipment_value_to_units(division.max_equipment).unwrap();

    assert_eq!(
        stockpile_before_units - stockpile_after_units,
        division_equipment_units
    );
    assert_eq!(division_equipment_units, division_max_units);
}

// ─── Test I: 不正required_equipment ─────────────────────────────────────

#[test]
fn test_i_invalid_required_equipment_is_rejected_atomically_at_request_time() {
    for invalid in [f64::NAN, f64::INFINITY, -1.0, 2_000_000_000.0] {
        let military_registry = registry_with_required_equipment(invalid);
        let mut country = fresh_country(1, 20_000);
        country.treasury = 500.0;
        country
            .stockpile
            .set(ResourceType::MilitaryEquipment, 100.0);

        let result = request_recruitment(
            &mut country,
            &military_registry,
            DivisionDefinitionId(1),
            StateId(1),
        );
        assert!(
            result.is_err(),
            "invalid required_equipment={invalid} must be rejected"
        );
        assert_eq!(country.recruitment_queue.len(), 0);
        assert_eq!(country.available_manpower, 20_000);
        assert_eq!(country.mobilized_manpower, 0);
        assert_eq!(
            country.stockpile.get(ResourceType::MilitaryEquipment),
            100.0
        );
        assert_eq!(country.treasury, 500.0);
    }
}

// ─── Test J: 複数queue item(P27-001: 両方とも同日に完成する) ────────────
//
// P27-001以前は、備蓄150.0(必要100.0×2件分には不足)に対して2件同時投入すると、
// 1件目(item A)がall-or-nothingで全額100.0を消費し、2件目(item B)は完成できず
// 待機し続けた。P27-001の候補Cでは、`process_recruitment`が`recruitment_queue`
// をVecの順序どおり逐次処理し、各項目の完成時点でその時点の備蓄を都度読み直す
// ため、item Aが100.0(満額)を消費した"後"にitem Bが評価され、残り50.0を
// 部分支給で受け取って**同日に両方とも完成する**。これは新しいHamilton配分等を
// 導入したものではなく、既存のVec順逐次処理がそのまま自然に決定的なFIFO配分を
// 実現している(P27-001報告書§7参照)。

#[test]
fn test_j_multiple_queue_items_both_complete_same_day_first_full_then_remainder() {
    let mut military_registry = setup_registry(); // required_equipment=100.0
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 200_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 150.0);

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap(); // item A
    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap(); // item B
    country_registry.countries.push(country);

    run_days(&mut country_registry, &mut military_registry, 30);

    assert_eq!(
        military_registry.divisions.len(),
        2,
        "P27-001: both queue items complete the same day, the second with the remainder"
    );
    assert_eq!(
        country_registry.countries[0]
            .stockpile
            .get(ResourceType::MilitaryEquipment),
        0.0,
        "all 150.0 must have been distributed across the two completions"
    );
    assert_eq!(
        country_registry.countries[0].recruitment_queue.len(),
        0,
        "no item remains waiting"
    );
    let mut equipments: Vec<f64> = military_registry.divisions.values().map(|d| d.equipment).collect();
    equipments.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        equipments,
        vec![50.0, 100.0],
        "the first item in queue order receives the full amount, the second the remainder"
    );
    for division in military_registry.divisions.values() {
        assert_eq!(
            division.max_equipment, 100.0,
            "max_equipment stays at the full requirement regardless of how much was issued"
        );
    }
}

// ─── Test K: 複数国家 ────────────────────────────────────────────────────

#[test]
fn test_k_multiple_countries_do_not_share_equipment_or_manpower() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();
    let mut country_a = fresh_country(1, 20_000);
    // country_aのstockpileは既定の0のまま(不足)。
    let mut country_b = fresh_country(2, 20_000);
    country_b
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    request_recruitment(
        &mut country_a,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    request_recruitment(
        &mut country_b,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(2),
    )
    .unwrap();
    country_registry.countries.push(country_a);
    country_registry.countries.push(country_b);

    run_days(&mut country_registry, &mut military_registry, 30);

    // P27-001: country Aも(装備0のまま)同日に完成する——country間のEquipment・
    // manpower分離自体は変わらず維持される(それぞれ独立に0/100を支給される)。
    assert_eq!(
        military_registry.divisions.len(),
        2,
        "P27-001: country A's shortage no longer blocks its own completion"
    );
    let division_a = military_registry
        .divisions
        .values()
        .find(|d| d.owner == CountryId(1))
        .unwrap();
    assert_eq!(division_a.equipment, 0.0, "country A had nothing to issue");
    assert_eq!(division_a.max_equipment, 100.0);
    let division_b = military_registry
        .divisions
        .values()
        .find(|d| d.owner == CountryId(2))
        .unwrap();
    assert_eq!(division_b.equipment, 100.0, "country B had the full amount");
    assert_eq!(
        country_registry
            .get(CountryId(1))
            .unwrap()
            .recruitment_queue
            .len(),
        0,
        "country A's queue is also empty now"
    );
    assert_eq!(
        country_registry
            .get(CountryId(1))
            .unwrap()
            .stockpile
            .get(ResourceType::MilitaryEquipment),
        0.0,
        "country A's stockpile was already 0 and stays 0 (no cross-country leakage)"
    );
    assert_eq!(
        country_registry
            .get(CountryId(2))
            .unwrap()
            .stockpile
            .get(ResourceType::MilitaryEquipment),
        0.0
    );
}

// ─── Test M: treasury ────────────────────────────────────────────────────
//
// `DivisionDefinition`にはrequired_equipment以外の正式な募兵費用フィールドは
// 存在しない(`maintenance_cost`は建物の月次維持費計算[`economy::finance`]専用で
// あり、Division募兵時には一切参照されないことをコード監査で確認済み)。したがって
// treasuryは、stockpileの充足状況に関わらず募兵によって一切変化しない。

#[test]
fn test_m_treasury_is_never_touched_by_recruitment_regardless_of_equipment_stockpile() {
    for stockpile_value in [0.0, 50.0, 100.0, 500.0] {
        let mut military_registry = setup_registry();
        let mut country_registry = CountryRegistry::default();
        let mut country = fresh_country(1, 20_000);
        country.treasury = 12_345.0;
        country
            .stockpile
            .set(ResourceType::MilitaryEquipment, stockpile_value);

        request_recruitment(
            &mut country,
            &military_registry,
            DivisionDefinitionId(1),
            StateId(1),
        )
        .unwrap();
        country_registry.countries.push(country);
        run_days(&mut country_registry, &mut military_registry, 30);

        assert_eq!(
            country_registry.countries[0].treasury, 12_345.0,
            "treasury changed unexpectedly for stockpile={stockpile_value}"
        );
    }
}

// ─── Test R: 消滅国家 ────────────────────────────────────────────────────

#[test]
fn test_r_eliminated_country_neither_consumes_equipment_nor_spawns_a_division() {
    use strategy_game::country::CountryStatus;

    let military_registry = setup_registry();
    let mut country = fresh_country(1, 20_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);
    country.status = CountryStatus::Eliminated {
        eliminated_date: "1936/01/01".to_string(),
        annexed_by: CountryId(9),
    };

    let result = request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    );
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
    assert_eq!(
        country.stockpile.get(ResourceType::MilitaryEquipment),
        100.0
    );
    assert_eq!(country.available_manpower, 20_000);
}

// ─── Test S: 未解決definition ────────────────────────────────────────────

#[test]
fn test_s_unresolved_definition_leaves_all_accounting_values_unchanged() {
    let military_registry = setup_registry(); // DivisionDefinitionId(1)のみ定義
    let mut country = fresh_country(1, 20_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    let result = request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(999), // 未定義
        StateId(1),
    );
    assert!(result.is_err());
    assert_eq!(country.recruitment_queue.len(), 0);
    assert_eq!(country.available_manpower, 20_000);
    assert_eq!(country.mobilized_manpower, 0);
    assert_eq!(
        country.stockpile.get(ResourceType::MilitaryEquipment),
        100.0
    );
}

// ─── Test U: 完成日を境にした備蓄のビット精度な変化(P27-001で更新) ──────
//
// P27-001以前は42.5 < 必要100.0が「完成させない」ことを意味したため、40日間
// stockpileがビット単位で不変であることを検証していた。P27-001では、
// `recruitment_days`(このfixtureでは30日)に到達した時点で42.5全額が部分支給
// され、stockpileは0.0へ変化する。「完成日より前は完全に不変」「完成日に
// ちょうど1回だけ変化」「完成日より後は(queueが空なので)再び不変」という
// 3段階を、ビット精度で検証する。

#[test]
fn test_u_stockpile_changes_exactly_once_on_the_completion_day_then_stays_bit_stable() {
    let mut military_registry = setup_registry(); // required_equipment=100.0, recruitment_days=30
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country.stockpile.set(ResourceType::MilitaryEquipment, 42.5);
    let stockpile_bits_before = 42.5f64.to_bits();

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    country_registry.countries.push(country);

    // 完成日(30日目)より前(1〜29日目)は、queueがまだdays_remaining>0のため
    // stockpileはビット単位で不変。
    for day in 1..30 {
        process_recruitment(&mut country_registry, &mut military_registry);
        assert_eq!(
            country_registry.countries[0]
                .stockpile
                .get(ResourceType::MilitaryEquipment)
                .to_bits(),
            stockpile_bits_before,
            "stockpile must be bit-identical before the completion day (day {day})"
        );
    }

    // 30日目(完成日): 42.5全額が部分支給され、stockpileは0.0になる。
    process_recruitment(&mut country_registry, &mut military_registry);
    assert_eq!(
        country_registry.countries[0]
            .stockpile
            .get(ResourceType::MilitaryEquipment),
        0.0,
        "P27-001: the full 42.5 must be issued on the completion day, not withheld"
    );
    assert_eq!(military_registry.divisions.len(), 1);
    let division = military_registry.divisions.values().next().unwrap();
    assert_eq!(division.equipment, 42.5);
    assert_eq!(division.max_equipment, 100.0);

    // 完成後(queueが空)は、再びビット単位で不変(0.0のまま)。
    let stockpile_bits_after_completion = 0.0f64.to_bits();
    for _ in 0..10 {
        process_recruitment(&mut country_registry, &mut military_registry);
        assert_eq!(
            country_registry.countries[0]
                .stockpile
                .get(ResourceType::MilitaryEquipment)
                .to_bits(),
            stockpile_bits_after_completion
        );
    }
}

// ─── Test V: 既存manpower回帰 ────────────────────────────────────────────

#[test]
fn test_v_existing_manpower_numbers_are_unchanged_when_equipment_is_sufficient() {
    let mut military_registry = setup_registry();
    let mut country_registry = CountryRegistry::default();
    let mut country = fresh_country(1, 20_000);
    country
        .stockpile
        .set(ResourceType::MilitaryEquipment, 100.0);

    request_recruitment(
        &mut country,
        &military_registry,
        DivisionDefinitionId(1),
        StateId(1),
    )
    .unwrap();
    // P23時点から不変の即時人的資源遷移(§3)。
    assert_eq!(country.available_manpower, 10_000);
    assert_eq!(country.mobilized_manpower, 10_000);

    country_registry.countries.push(country);
    run_days(&mut country_registry, &mut military_registry, 30);
    assert_eq!(country_registry.countries[0].available_manpower, 10_000);
    assert_eq!(country_registry.countries[0].mobilized_manpower, 10_000);
}

// ─── 実App用ハーネス(Test L・N・O・P・Q・T) ────────────────────────────

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "strategy_game_p25_003_e2e_{label}_{}_{nanos}_{n}",
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
    app.update(); // Startup: 実データ読込 + CountrySelectionへ遷移

    app.insert_resource(PlayerCountry(Some(player)));
    app.insert_state(GameState::Playing);
    app.update(); // OnEnter(Playing)

    app
}

fn send_save_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<SaveRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(SaveRequestMessage);
    state.apply(app.world_mut());
}

fn send_load_request(app: &mut App) {
    let mut state: SystemState<MessageWriter<LoadRequestMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(LoadRequestMessage);
    state.apply(app.world_mut());
}

fn send_day_changed(app: &mut App, year: i32, month: u8, day: u8) {
    let mut state: SystemState<MessageWriter<DayChangedMessage>> =
        SystemState::new(app.world_mut());
    state
        .get_mut(app.world_mut())
        .expect("writer")
        .write(DayChangedMessage { year, month, day });
    state.apply(app.world_mut());
    app.update();
}

fn set_military_equipment_stockpile(app: &mut App, country_id: CountryId, value: f64) {
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .get_mut(country_id)
        .unwrap()
        .stockpile
        .set(ResourceType::MilitaryEquipment, value);
}

fn military_equipment_stockpile(app: &App, country_id: CountryId) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .get(country_id)
        .unwrap()
        .stockpile
        .get(ResourceType::MilitaryEquipment)
}

fn call_request_recruitment(
    app: &mut App,
    country_id: CountryId,
    division_id: DivisionDefinitionId,
    target_state: StateId,
) -> Result<(), &'static str> {
    let mut state: SystemState<(ResMut<CountryRegistry>, Res<MilitaryRegistry>)> =
        SystemState::new(app.world_mut());
    let (mut country_registry, military_registry) =
        state.get_mut(app.world_mut()).expect("system params");
    let country = country_registry.get_mut(country_id).unwrap();
    let result = request_recruitment(country, &military_registry, division_id, target_state);
    state.apply(app.world_mut());
    result
}

// ─── Test L: 同日生産装備 ────────────────────────────────────────────────

#[test]
fn test_l_same_day_produced_equipment_can_be_used_by_the_same_days_recruitment() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_l");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // 標準Infantry定義を取得し、その必要量ちょうどを月初生産で追加する。
    let required_equipment = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment
    };

    // 日初めはstockpile不足。
    set_military_equipment_stockpile(&mut app, player, 0.0);
    assert!(military_equipment_stockpile(&app, player) < required_equipment);

    // Economyが生産した分をこのSystem順序より前に反映済みとして扱う
    // (実際のMilitaryFactory生産は月次のため、テストでは同日にEconomy相当の
    // 生産が完了済みの状態を直接再現する——DailySimulationSet::Economyが
    // MilitaryActionより先に走るという既存のP25-001監査済み順序をそのまま前提とする)。
    set_military_equipment_stockpile(&mut app, player, required_equipment);

    call_request_recruitment(
        &mut app,
        player,
        strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID,
        StateId(0),
    )
    .unwrap();

    // recruitment_daysぶん日を進めて完了させる。
    let recruitment_days = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    for day in 0..recruitment_days {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }

    assert_eq!(
        military_equipment_stockpile(&app, player),
        0.0,
        "the same-day production must have been available for this recruitment"
    );
}

// ─── Test N: 初期Division ────────────────────────────────────────────────

#[test]
fn test_n_new_game_initial_divisions_do_not_debit_the_stockpile() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_n");
    let app = setup_app_in_playing(player, temp_dir.save_file_config());

    let military_registry = app.world().resource::<MilitaryRegistry>();
    assert!(
        !military_registry.divisions.is_empty(),
        "New Game must have spawned initial divisions"
    );
    assert_eq!(
        military_equipment_stockpile(&app, player),
        0.0,
        "initial divisions must not have debited the MilitaryEquipment stockpile"
    );
}

// ─── Test O: Loaded Game Division ────────────────────────────────────────

#[test]
fn test_o_loaded_game_divisions_do_not_redebit_the_stockpile() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_o");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_military_equipment_stockpile(&mut app, player, 42.0);
    let division_count_before = app.world().resource::<MilitaryRegistry>().divisions.len();

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        military_equipment_stockpile(&app, player),
        42.0,
        "loading pre-existing divisions must not re-debit the stockpile"
    );
    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before,
        "loading must not duplicate divisions"
    );
}

// ─── Test P: 0装備完成のsave/load往復(P27-001で全面刷新) ────────────────
//
// P27-001以前はここで備蓄0のqueueが完成せずに待機し続けることを検証していたが、
// P27-001の候補C採用によりこのシナリオ自体が「即座に0装備で完成する」に変わった
// (旧テストの前提が成立しなくなった、意図的な契約の反転)。本テストは新しい
// 正規挙動——0装備Divisionが実App経由で即座に完成し、save→loadを跨いでも
// 再生成・再消費されずに1件のまま安定する——を検証する。

#[test]
fn test_p_zero_equipment_division_completes_immediately_and_round_trips_through_save_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_p");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    set_military_equipment_stockpile(&mut app, player, 0.0);
    let division_count_before_request = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == player)
        .count();
    call_request_recruitment(
        &mut app,
        player,
        strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID,
        StateId(0),
    )
    .unwrap();

    let recruitment_days = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    for day in 0..recruitment_days {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }

    let division_count_after_training = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .filter(|d| d.owner == player)
        .count();
    assert_eq!(
        division_count_after_training,
        division_count_before_request + 1,
        "P27-001: training completion must not be delayed by an empty equipment stockpile"
    );
    assert!(
        app.world()
            .resource::<CountryRegistry>()
            .get(player)
            .unwrap()
            .recruitment_queue
            .is_empty(),
        "the completed item must be removed from the queue"
    );
    let new_division = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .values()
        .find(|d| d.owner == player && d.equipment == 0.0)
        .expect("a zero-equipped division must exist")
        .clone();
    assert!(new_division.max_equipment > 0.0, "max_equipment must retain the full requirement");
    let total_division_count_before_save = app.world().resource::<MilitaryRegistry>().divisions.len();

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        total_division_count_before_save,
        "save/load must not duplicate or drop the zero-equipped division"
    );
    let reloaded = app
        .world()
        .resource::<MilitaryRegistry>()
        .divisions
        .get(&new_division.id)
        .expect("the same division id must survive the round trip");
    assert_eq!(reloaded.equipment, 0.0);
    assert_eq!(reloaded.max_equipment, new_division.max_equipment);
    assert_eq!(military_equipment_stockpile(&app, player), 0.0);
}

// ─── Test Q: completed save/load ─────────────────────────────────────────

#[test]
fn test_q_completed_recruitment_round_trips_through_save_and_load() {
    let player = CountryId(0);
    let temp_dir = TempTestDir::new("test_q");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    let required_equipment = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .required_equipment
    };
    set_military_equipment_stockpile(&mut app, player, required_equipment);
    call_request_recruitment(
        &mut app,
        player,
        strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID,
        StateId(0),
    )
    .unwrap();

    let recruitment_days = {
        let military_registry = app.world().resource::<MilitaryRegistry>();
        military_registry
            .definitions
            .get(&strategy_game::military::data::STANDARD_INFANTRY_DEFINITION_ID)
            .unwrap()
            .recruitment_days
    };
    for day in 0..recruitment_days {
        send_day_changed(&mut app, 1800, 1, (day % 28 + 1) as u8);
    }

    let division_count_before = app.world().resource::<MilitaryRegistry>().divisions.len();
    let stockpile_before = military_equipment_stockpile(&app, player);

    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();

    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before
    );
    assert_eq!(military_equipment_stockpile(&app, player), stockpile_before);

    // 再ロードしても再消費されない。
    send_save_request(&mut app);
    app.update();
    send_load_request(&mut app);
    app.update();
    app.update();
    assert_eq!(
        app.world().resource::<MilitaryRegistry>().divisions.len(),
        division_count_before
    );
    assert_eq!(military_equipment_stockpile(&app, player), stockpile_before);
}

// ─── Test T: AI経路(P27-001: 「同じゲート」から「同じ完成規則」へ) ─────
//
// P27-001以前は、AIが備蓄不足のまま募兵をqueueへ入れても完成しない(=同じ
// all-or-nothingゲートをプレイヤーと共有する)ことを検証していた。P27-001は
// この完成規則自体を変更した(候補C)ため、本テストはAIも新しい正規挙動——
// 備蓄0でも即座に(0装備で)完成する——をプレイヤーと全く同じ関数経由で
// 受け取ることを検証する形へ更新する。

#[test]
fn test_t_ai_recruitment_receives_the_same_equipment_completion_rule_as_the_player() {
    let player = CountryId(0);
    let ai_country = CountryId(1);
    let temp_dir = TempTestDir::new("test_t");
    let mut app = setup_app_in_playing(player, temp_dir.save_file_config());

    // AI国家のstockpileを不足させ、queueが空であることを確認してから月替わりを送る。
    set_military_equipment_stockpile(&mut app, ai_country, 0.0);
    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let ai = country_registry.get_mut(ai_country).unwrap();
        assert!(
            ai.recruitment_queue.is_empty(),
            "precondition: AI queue must start empty"
        );
    }

    let mut state: SystemState<MessageWriter<strategy_game::app::time::MonthChangedMessage>> =
        SystemState::new(app.world_mut());
    state.get_mut(app.world_mut()).expect("writer").write(
        strategy_game::app::time::MonthChangedMessage {
            year: 1800,
            month: 2,
        },
    );
    state.apply(app.world_mut());
    app.update();

    // AIが募兵を判断してキューへ投入した場合(装備不足でも投入自体は成功する)、
    // P27-001以降は0装備のままでも訓練完了と同時に完成する。
    let ai_queue_is_non_empty = !app
        .world()
        .resource::<CountryRegistry>()
        .get(ai_country)
        .unwrap()
        .recruitment_queue
        .is_empty();
    if ai_queue_is_non_empty {
        let division_count_before = app
            .world()
            .resource::<MilitaryRegistry>()
            .divisions
            .values()
            .filter(|d| d.owner == ai_country)
            .count();

        for day in 1..=35u8 {
            send_day_changed(&mut app, 1800, 2, (day - 1) % 28 + 1);
        }

        let military_registry = app.world().resource::<MilitaryRegistry>();
        let division_count_after = military_registry
            .divisions
            .values()
            .filter(|d| d.owner == ai_country)
            .count();
        assert_eq!(
            division_count_before + 1,
            division_count_after,
            "P27-001: AI must complete recruitment with whatever equipment is available, \
             the same completion rule the player receives"
        );
        let new_division = military_registry
            .divisions
            .values()
            .find(|d| d.owner == ai_country)
            .expect("the AI's new division must exist");
        assert_eq!(
            new_division.equipment, 0.0,
            "the AI's stockpile was insufficient, so the division must be issued zero equipment"
        );
        assert!(
            new_division.max_equipment > 0.0,
            "max_equipment must retain the full requirement for later reinforcement"
        );
    }
}
