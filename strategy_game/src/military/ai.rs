//! P23-009: AI国家の月次自動募兵。
//!
//! 既存の`military::recruitment::request_recruitment`を唯一の書き込み経路として
//! 再利用する。本モジュール自身は`available_manpower`・`mobilized_manpower`・
//! `treasury`・`recruitment_queue`・Division生成のいずれも直接変更しない
//! (判断結果を`request_recruitment`へ渡すだけ)。
//!
//! 対象は`STANDARD_INFANTRY_DEFINITION_ID`(New Game初期配置・プレイヤーUI募兵と
//! 共通の正規Infantry Definition)のみ。目標Division数は人的資源基準上限の50%、
//! 募兵後も基準上限の25%を`available_manpower`として残せる場合だけ実行する
//! (§5/§6の定数はいずれも「平時・戦時共通の保守的な軍備水準」という意図のみで、
//! 戦争状態や国力による補正は行わない)。
use crate::app::time::GameDate;
use crate::building::data::BuildingRegistry;
use crate::common::{CountryId, StateId};
use crate::country::country_ai::CountryAiRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::military::data::{DivisionStatus, MilitaryRegistry, STANDARD_INFANTRY_DEFINITION_ID};
use crate::military::manpower::compute_manpower_ledger;
use crate::military::production_capacity_gate::evaluate_production_capacity_gate;
use crate::military::recruitment::request_recruitment;
use crate::military::recruitment_supply_gate::evaluate_recruitment_supply_gate;
use crate::population::growth::{compute_manpower_cap, compute_population_by_owner};
use crate::state::data::{StateData, StateRegistry};
use crate::war::data::WarRegistry;
use bevy::log::debug;
use bevy::prelude::*;
use std::collections::HashMap;

/// 国家単位のAI月次募兵判断の結果。テストが正規判断経路(`decide_and_recruit_for_
/// country`)の結果を、状態を直接書き換えずに検証できるようにする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiRecruitmentOutcome {
    Recruited,
    SkippedTargetMet,
    SkippedQueueNotEmpty,
    SkippedInsufficientManpower,
    SkippedNoControlledState,
    SkippedLedgerInvalid,
    SkippedMissingDefinition,
    /// P26-008-A: MilitarySupplies補給ゲートにより新規募兵を見送った
    /// (`SkippedInsufficientManpower`等の既存理由へ偽装しない、専用の観測可能な
    /// 理由)。国家備蓄不足・bridge不足・既存Divisionの配送不全・補給ゲート自体の
    /// 評価失敗(防御的な分岐、安全側として見送る)のいずれかが原因――
    /// `SkippedLedgerInvalid`が複数の帳簿不整合原因を1種類にまとめているのと
    /// 同じ設計判断であり、公開enumを不必要に細分化しない。
    SkippedSupplyShortage,
    /// P29-004: 新しい師団の完成後の日次需要(生存師団・訓練中・今回の募兵)を、直近の月次
    /// 生産の実績から再構成した持続的な産出で賄えないため、新規募兵を見送った(備蓄が
    /// 潤沢でも許可しない)。評価自体の失敗(overflow 等)も安全側として見送る。
    SkippedProductionCapacity,
    /// 呼び出し時点で対象国家が`CountryRegistry`に見当たらない防御的なケース
    /// (通常の呼び出し経路[`evaluate_ai_monthly_recruitment`]では起こり得ない)。
    SkippedCountryNotFound,
}

/// AI国家の目標Division数(人的資源基準上限の50%が目標)。
///
/// `required_manpower==0`または`cap<required_manpower`では0。それ以外で
/// `floor(cap/2)/required_manpower`が0になる場合(=cap>=requiredだが1個分の
/// 半分未満)でも、最低1個を目標とする。`cap`を先に2で割ってから
/// `required_manpower`で割ることで、`2 * required_manpower`のoverflowを
/// 経由せずに計算する。
pub fn calculate_ai_target_divisions(cap: u64, required_manpower: u64) -> u64 {
    if required_manpower == 0 || cap < required_manpower {
        return 0;
    }
    let target = (cap / 2) / required_manpower;
    if target == 0 { 1 } else { target }
}

/// AI国家が募兵後も維持する`available_manpower`の予備(基準上限の25%、切り上げ)。
pub fn calculate_ai_manpower_reserve(cap: u64) -> u64 {
    let base = cap / 4;
    if !cap.is_multiple_of(4) {
        base + 1
    } else {
        base
    }
}

/// 対象国家が現在保有する「戦力」を数える: 生存Division数 + 募兵キュー件数
/// (P23-008の自動補充が担当する損耗Divisionは、生存している限り1個として数え、
/// 不足数として二重に扱わない)。
fn current_force_count(
    living_by_owner: &HashMap<CountryId, u64>,
    queue_len: usize,
    country_id: CountryId,
) -> u64 {
    living_by_owner.get(&country_id).copied().unwrap_or(0) + queue_len as u64
}

/// `MilitaryRegistry`を1回走査し、国家owner別の生存Division数を集計する
/// (`manpower>0 && status!=Destroyed`、Army数は使わない)。
fn count_living_divisions_by_owner(
    military_registry: &MilitaryRegistry,
) -> HashMap<CountryId, u64> {
    let mut counts: HashMap<CountryId, u64> = HashMap::new();
    for division in military_registry.divisions.values() {
        if division.manpower > 0 && division.status != DivisionStatus::Destroyed {
            *counts.entry(division.owner).or_insert(0) += 1;
        }
    }
    counts
}

/// AI募兵のtarget stateを選ぶ: 自国所有かつ自国支配の首都を優先し、使えない場合は
/// 自国所有・自国支配の州のうちpopulationが最大(同率ならStateId最小)のものを選ぶ。
/// `StateRegistry.states`(`Vec`)を1回走査するだけで、`HashMap`走査順には依存しない。
pub fn select_ai_recruitment_state(
    country_id: CountryId,
    capital: StateId,
    state_registry: &StateRegistry,
) -> Option<StateId> {
    if let Some(capital_state) = state_registry.get(capital)
        && capital_state.owner_country_id == country_id
        && capital_state.controller() == country_id
    {
        return Some(capital);
    }

    let is_controlled_by =
        |s: &&StateData| s.owner_country_id == country_id && s.controller() == country_id;
    state_registry
        .states
        .iter()
        .filter(is_controlled_by)
        .max_by(|a, b| a.population.cmp(&b.population).then(b.id.0.cmp(&a.id.0)))
        .map(|s| s.id)
}

/// 1国家分のAI月次募兵判断を行い、条件を満たす場合のみ`request_recruitment`を
/// 呼び出す。この関数自身は`available_manpower`・`mobilized_manpower`・
/// `treasury`・`recruitment_queue`・Divisionを一切直接変更しない。
#[allow(clippy::too_many_arguments)]
fn decide_and_recruit_for_country(
    country_registry: &mut CountryRegistry,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
    building_registry: &BuildingRegistry,
    war_registry: &WarRegistry,
    game_date: &GameDate,
    population_by_owner: &HashMap<CountryId, u64>,
    living_by_owner: &HashMap<CountryId, u64>,
    ai_state: &mut crate::country::country_ai::CountryAiState,
    country_id: CountryId,
) -> AiRecruitmentOutcome {
    let Some(def) = military_registry
        .definitions
        .get(&STANDARD_INFANTRY_DEFINITION_ID)
    else {
        debug!(
            "[AiRecruitment] standard infantry definition unavailable, skipping this month for all countries"
        );
        return AiRecruitmentOutcome::SkippedMissingDefinition;
    };
    let required = def.required_manpower;

    let Some(country) = country_registry.get(country_id) else {
        return AiRecruitmentOutcome::SkippedCountryNotFound;
    };

    if !country.recruitment_queue.is_empty() {
        return AiRecruitmentOutcome::SkippedQueueNotEmpty;
    }

    let ledger = match compute_manpower_ledger(country, military_registry) {
        Ok(ledger) => ledger,
        Err(_) => {
            debug!(
                "[AiRecruitment] country {country_id:?}: manpower ledger unavailable, skipping this month"
            );
            return AiRecruitmentOutcome::SkippedLedgerInvalid;
        }
    };
    if ledger.unaccounted_living_excess != 0 {
        debug!(
            "[AiRecruitment] country {country_id:?}: manpower ledger inconsistent \
             (unaccounted_living_excess={}), skipping this month",
            ledger.unaccounted_living_excess
        );
        return AiRecruitmentOutcome::SkippedLedgerInvalid;
    }

    let population = population_by_owner.get(&country_id).copied().unwrap_or(0);
    let cap = compute_manpower_cap(population);
    let target = calculate_ai_target_divisions(cap, required);
    let current = current_force_count(living_by_owner, country.recruitment_queue.len(), country_id);
    if current >= target {
        return AiRecruitmentOutcome::SkippedTargetMet;
    }

    // P26-008-A: 補給ゲート。既存の目標未達判定(直前)が確定し、募兵候補
    // Definition(`def`、この関数冒頭で取得済み)も確定した後、既存の人的資源条件
    // (直後)より前に評価する(spec §14の推奨順序)。`ai_state.recruitment_supply_
    // blocked`は「今月、新規募兵候補があるすべてのAI国家」について毎回更新するため、
    // 状態が古いまま放置されることはない(この関数はキュー非空・目標達成済みの
    // 国家では既に上のreturnで抜けており、そこでは更新しない――「新規募兵を
    // 検討する条件が整った月だけ更新する」という一貫した基準)。
    match evaluate_recruitment_supply_gate(
        country,
        military_registry,
        war_registry,
        def,
        game_date,
        ai_state.recruitment_supply_blocked,
    ) {
        Ok(new_blocked) => {
            ai_state.recruitment_supply_blocked = new_blocked;
            if new_blocked {
                return AiRecruitmentOutcome::SkippedSupplyShortage;
            }
        }
        Err(_) => {
            // 防御的な分岐(通常のゲーム状態では到達しない、spec §17に列挙された
            // Equipment不足によるqueue停滞とは無関係)。安全側としてこの月の新規
            // 募兵を見送るが、ゲート状態自体は変更しない(不完全な評価結果で
            // 中間帯のヒステリシス状態を書き換えない)。
            debug!(
                "[AiRecruitment] country {country_id:?}: supply gate evaluation failed, \
                 skipping this month as a safe default"
            );
            return AiRecruitmentOutcome::SkippedSupplyShortage;
        }
    }

    // P29-004: 生産能力ゲート(補給ゲートの後に追加する。既存の判定・ヒステリシス状態は
    // 変更しない)。訓練キュー(この月に先に許可した注文を含む)は将来需要に算入される。
    match evaluate_production_capacity_gate(
        country,
        military_registry,
        state_registry,
        building_registry,
        def,
    ) {
        Ok(verdict) if verdict.allowed => {}
        Ok(verdict) => {
            debug!(
                "[AiRecruitment] country {country_id:?}: production capacity gate holds recruitment \n                 (capacity {} < required {} units/month, unreported MF entries {})",
                verdict.capacity_monthly_units,
                verdict.required_monthly_units,
                verdict.unreported_entries
            );
            return AiRecruitmentOutcome::SkippedProductionCapacity;
        }
        Err(_) => {
            debug!(
                "[AiRecruitment] country {country_id:?}: production capacity gate evaluation \n                 failed, skipping this month as a safe default"
            );
            return AiRecruitmentOutcome::SkippedProductionCapacity;
        }
    }

    let reserve = calculate_ai_manpower_reserve(cap);
    let Some(needed) = required.checked_add(reserve) else {
        return AiRecruitmentOutcome::SkippedInsufficientManpower;
    };
    if country.available_manpower < needed {
        return AiRecruitmentOutcome::SkippedInsufficientManpower;
    }

    let capital = country.capital_state_id;
    let Some(target_state) = select_ai_recruitment_state(country_id, capital, state_registry)
    else {
        return AiRecruitmentOutcome::SkippedNoControlledState;
    };

    let Some(country_mut) = country_registry.get_mut(country_id) else {
        return AiRecruitmentOutcome::SkippedCountryNotFound;
    };
    match request_recruitment(
        country_mut,
        military_registry,
        STANDARD_INFANTRY_DEFINITION_ID,
        target_state,
    ) {
        Ok(()) => AiRecruitmentOutcome::Recruited,
        // 直前の事前チェックと矛盾する状態変化はここまでの検証で起こり得ないが、
        // request_recruitment自身の最終検証が拒否した場合は安全側に倒す。
        Err(_) => AiRecruitmentOutcome::SkippedInsufficientManpower,
    }
}

/// P23-009のエントリーポイント(Bevy非依存)。プレイヤー以外の活動国家を
/// `CountryId`昇順で処理し、各国最大1件までAI月次募兵を試みる。国家A失敗が
/// 国家Bの判断を妨げないよう、国家ごとに独立して評価する。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_ai_monthly_recruitment(
    country_registry: &mut CountryRegistry,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
    building_registry: &BuildingRegistry,
    war_registry: &WarRegistry,
    game_date: &GameDate,
    country_ai_registry: &mut CountryAiRegistry,
    player_country: Option<CountryId>,
) {
    let population_by_owner = compute_population_by_owner(state_registry);
    let living_by_owner = count_living_divisions_by_owner(military_registry);

    let mut candidates: Vec<CountryId> = country_registry
        .countries
        .iter()
        .filter(|c| c.is_active() && Some(c.id) != player_country)
        .map(|c| c.id)
        .collect();
    candidates.sort_by_key(|id| id.0);

    for country_id in candidates {
        let ai_state = country_ai_registry.get_or_create_mut(country_id);
        decide_and_recruit_for_country(
            country_registry,
            military_registry,
            state_registry,
            building_registry,
            war_registry,
            game_date,
            &population_by_owner,
            &living_by_owner,
            ai_state,
            country_id,
        );
    }
}

/// System: `MonthChangedMessage`が発生したフレームにつき最大1回だけ
/// `evaluate_ai_monthly_recruitment`を実行する(`diplomacy::ai_claim_generation::
/// handle_monthly_ai_claim_generation`と同じ二重処理防止パターン)。
/// `DailySimulationSet::CountryAi`(Economyより後、UiUpdateより前)に配置する。
#[allow(clippy::too_many_arguments)]
pub fn handle_monthly_ai_recruitment(
    mut month_events: MessageReader<crate::app::time::MonthChangedMessage>,
    mut country_registry: ResMut<CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    state_registry: Res<StateRegistry>,
    building_registry: Res<BuildingRegistry>,
    war_registry: Res<WarRegistry>,
    game_date: Res<GameDate>,
    mut country_ai_registry: ResMut<CountryAiRegistry>,
    player_country: Res<PlayerCountry>,
) {
    let mut evaluated_this_frame = false;
    for _event in month_events.read() {
        if evaluated_this_frame {
            continue;
        }
        evaluated_this_frame = true;

        evaluate_ai_monthly_recruitment(
            &mut country_registry,
            &military_registry,
            &state_registry,
            &building_registry,
            &war_registry,
            &game_date,
            &mut country_ai_registry,
            player_country.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
    use crate::country::country_ai::CountryAiRegistry;
    use crate::country::{CountryData, CountryStatus};
    use crate::economy::resources::ResourceType;
    use crate::military::data::{Division, DivisionDefinition, DivisionSize, DivisionType};
    use crate::military::recruitment::RecruitmentQueueItem;

    // ── §14.1 目標Division数 ──────────────────────────────────────────

    #[test]
    fn target_divisions_examples() {
        assert_eq!(calculate_ai_target_divisions(100_000, 0), 0);
        assert_eq!(calculate_ai_target_divisions(9_999, 10_000), 0);
        assert_eq!(calculate_ai_target_divisions(10_000, 10_000), 1);
        assert_eq!(calculate_ai_target_divisions(19_999, 10_000), 1);
        assert_eq!(calculate_ai_target_divisions(20_000, 10_000), 1);
        assert_eq!(calculate_ai_target_divisions(40_000, 10_000), 2);
        assert_eq!(calculate_ai_target_divisions(100_000, 10_000), 5);
    }

    #[test]
    fn target_divisions_does_not_overflow_at_u64_max() {
        let target = calculate_ai_target_divisions(u64::MAX, 1);
        assert_eq!(target, u64::MAX / 2);
    }

    // 「目標計算に現在Division数・treasury・戦争状態を混ぜない」ことは、
    // この関数のシグネチャ自体が(cap, required_manpower)しか受け取らないことで
    // 型として保証される(呼び出し不能なテストではなく設計で示す)。

    // ── §14.2 人的資源予備 ────────────────────────────────────────────

    #[test]
    fn manpower_reserve_examples() {
        assert_eq!(calculate_ai_manpower_reserve(0), 0);
        assert_eq!(calculate_ai_manpower_reserve(1), 1);
        assert_eq!(calculate_ai_manpower_reserve(4), 1);
        assert_eq!(calculate_ai_manpower_reserve(5), 2);
        assert_eq!(calculate_ai_manpower_reserve(100_000), 25_000);
    }

    #[test]
    fn manpower_reserve_does_not_overflow_at_u64_max() {
        let reserve = calculate_ai_manpower_reserve(u64::MAX);
        assert_eq!(reserve, u64::MAX / 4 + 1);
    }

    // ── §14.4 募兵州選択 ──────────────────────────────────────────────

    fn state(id: usize, owner: usize, controller: Option<usize>, population: u64) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            controller_country: controller.map(CountryId),
            population,
            ..StateData::default()
        }
    }

    #[test]
    fn select_state_prefers_the_capital_when_owned_and_controlled() {
        let states = StateRegistry::build(vec![state(0, 1, None, 100), state(1, 1, None, 999_999)]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            Some(StateId(0))
        );
    }

    #[test]
    fn select_state_falls_back_when_the_capital_is_enemy_controlled() {
        let states = StateRegistry::build(vec![
            state(0, 1, Some(2), 100), // 首都だが敵支配
            state(1, 1, None, 500),
        ]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            Some(StateId(1))
        );
    }

    #[test]
    fn select_state_does_not_panic_when_the_capital_state_id_is_missing() {
        let states = StateRegistry::build(vec![state(1, 1, None, 500)]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(999), &states),
            Some(StateId(1))
        );
    }

    #[test]
    fn select_state_alternative_prefers_largest_population() {
        let states = StateRegistry::build(vec![
            state(0, 1, Some(2), 0), // 首都: 敵支配
            state(1, 1, None, 100),
            state(2, 1, None, 300),
        ]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            Some(StateId(2))
        );
    }

    #[test]
    fn select_state_breaks_population_ties_by_ascending_state_id() {
        let states = StateRegistry::build(vec![
            state(0, 1, Some(2), 0),
            state(5, 1, None, 200),
            state(2, 1, None, 200),
        ]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            Some(StateId(2))
        );
    }

    #[test]
    fn select_state_excludes_enemy_and_foreign_and_allied_controlled_states() {
        let states = StateRegistry::build(vec![
            state(0, 1, Some(2), 0),   // 首都: 敵支配
            state(1, 2, None, 999),    // 他国所有
            state(2, 1, Some(2), 999), // 自国所有だが敵支配
            state(3, 1, Some(3), 999), // 自国所有だが第三国(同盟国相当)支配
        ]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            None
        );
    }

    #[test]
    fn select_state_returns_none_when_no_state_is_eligible() {
        let states = StateRegistry::build(vec![state(0, 2, None, 100)]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &states),
            None
        );
    }

    #[test]
    fn select_state_result_does_not_depend_on_insertion_order() {
        let forward = StateRegistry::build(vec![
            state(0, 1, Some(2), 0),
            state(1, 1, None, 100),
            state(2, 1, None, 300),
        ]);
        let reversed = StateRegistry::build(vec![
            state(2, 1, None, 300),
            state(1, 1, None, 100),
            state(0, 1, Some(2), 0),
        ]);
        assert_eq!(
            select_ai_recruitment_state(CountryId(1), StateId(0), &forward),
            select_ai_recruitment_state(CountryId(1), StateId(0), &reversed),
        );
    }

    // ── §14.5 AI国家単位判断 ──────────────────────────────────────────

    fn infantry_def(required_manpower: u64, required_equipment: f64) -> DivisionDefinition {
        DivisionDefinition {
            id: STANDARD_INFANTRY_DEFINITION_ID,
            name: "Standard Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower,
            required_equipment,
            recruitment_days: 30,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn test_country(id: usize, available: u64, mobilized: u64, treasury: f64) -> CountryData {
        let mut country = CountryData {
            id: CountryId(id),
            capital_state_id: StateId(id),
            available_manpower: available,
            mobilized_manpower: mobilized,
            treasury,
            ..CountryData::default()
        };
        // P26-008-A: このモジュールの既存テストはいずれも募兵補給ゲート自体を
        // 検証する対象ではない(既存の目標Division数・人的資源予備・州選択等を
        // 検証する)。十分なMilitarySupplies備蓄(1個師団需要1.0/日に対し10,000日分)
        // を与え、新規ゲートが意図せずこれらを`SkippedSupplyShortage`へ変えない
        // ようにする――補給ゲート自体の境界値は`recruitment_supply_gate`モジュールの
        // 単体テストと、専用のE2Eテストファイルで検証する。
        country.stockpile.set(ResourceType::MilitarySupplies, 10_000.0);
        country
    }

    fn test_division(id: usize, owner: usize, manpower: u64, status: DivisionStatus) -> Division {
        Division {
            id: DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(owner),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: manpower.max(1),
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status,
            def_id: STANDARD_INFANTRY_DEFINITION_ID,
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    struct Harness {
        countries: CountryRegistry,
        military: MilitaryRegistry,
        states: StateRegistry,
        war: WarRegistry,
        buildings: BuildingRegistry,
        country_ai: CountryAiRegistry,
        game_date: GameDate,
    }

    fn harness(
        countries: Vec<CountryData>,
        divisions: Vec<Division>,
        states: Vec<StateData>,
        required_manpower: u64,
    ) -> Harness {
        let mut military = MilitaryRegistry::default();
        military.definitions.insert(
            STANDARD_INFANTRY_DEFINITION_ID,
            infantry_def(required_manpower, 100.0),
        );
        for d in divisions {
            military.divisions.insert(d.id, d);
        }
        Harness {
            countries: CountryRegistry { countries },
            military,
            states: StateRegistry::build(states),
            war: WarRegistry::default(),
            buildings: BuildingRegistry::default(),
            country_ai: CountryAiRegistry::default(),
            // 月半ば(境界日ではない)の任意の日付。既存テストはbridge計算の
            // 暦境界そのものを検証する対象ではない――境界値は
            // `recruitment_supply_gate`モジュールの単体テストで検証する。
            game_date: GameDate::new(1800, 6, 15),
        }
    }

    fn decide(h: &mut Harness, country_id: CountryId) -> AiRecruitmentOutcome {
        let population_by_owner = compute_population_by_owner(&h.states);
        let living_by_owner = count_living_divisions_by_owner(&h.military);
        let ai_state = h.country_ai.get_or_create_mut(country_id);
        decide_and_recruit_for_country(
            &mut h.countries,
            &h.military,
            &h.states,
            &h.buildings,
            &h.war,
            &h.game_date,
            &population_by_owner,
            &living_by_owner,
            ai_state,
            country_id,
        )
    }

    /// population=200,000 -> cap=20,000 -> target(required=10,000)=1。
    fn owned_state_with_population(id: usize, owner: usize, population: u64) -> StateData {
        state(id, owner, None, population)
    }

    #[test]
    fn recruits_when_below_target() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::Recruited
        );
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .recruitment_queue
                .len(),
            1
        );
    }

    #[test]
    fn does_not_recruit_once_target_is_met() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 10_000, 100_000.0)],
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)], // current=1==target(1)
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedTargetMet
        );
    }

    #[test]
    fn does_not_demobilize_when_over_target() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 20_000, 100_000.0)],
            vec![
                test_division(0, 1, 10_000, DivisionStatus::Idle),
                test_division(1, 1, 10_000, DivisionStatus::Idle),
            ], // current=2 > target(1)
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedTargetMet
        );
        assert_eq!(
            h.military.divisions.len(),
            2,
            "no demobilization must occur"
        );
    }

    #[test]
    fn does_not_recruit_when_the_queue_already_has_an_entry() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        h.countries
            .get_mut(CountryId(1))
            .unwrap()
            .recruitment_queue
            .push(RecruitmentQueueItem {
                division_id: STANDARD_INFANTRY_DEFINITION_ID,
                target_state: StateId(1),
                days_remaining: 10,
                total_days: 30,
            });
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedQueueNotEmpty
        );
    }

    #[test]
    fn does_not_recruit_when_it_would_breach_the_manpower_reserve() {
        // cap=20,000 -> reserve=5,000, required=10,000 -> needed=15,000
        let mut h = harness(
            vec![test_country(1, 14_999, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedInsufficientManpower
        );
    }

    #[test]
    fn recruits_exactly_at_the_manpower_reserve_boundary() {
        let mut h = harness(
            vec![test_country(1, 15_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::Recruited
        );
    }

    /// P25-003: `required_equipment`はもはや`treasury`から差し引かれる通貨ではなく、
    /// `MilitaryEquipment`備蓄が不足していてもキュー投入(`request_recruitment`経由)
    /// 自体は成功する——装備充足は`process_recruitment`でのDivision生成時にのみ
    /// 判定される(§8)。AIもプレイヤーと同じ`request_recruitment`を経由するため、
    /// 同じ挙動を受ける。
    #[test]
    fn queues_recruitment_even_when_military_equipment_stockpile_is_insufficient() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 99.0)], // stockpile[MilitaryEquipment]は既定の0のまま(必要100.0に対して不足)
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::Recruited
        );
    }

    #[test]
    fn eliminated_country_is_never_a_recruitment_candidate() {
        let mut h = harness(
            vec![CountryData {
                status: CountryStatus::Eliminated {
                    eliminated_date: "1936/01/01".to_string(),
                    annexed_by: CountryId(9),
                },
                ..test_country(1, 1_000_000, 0, 100_000.0)
            }],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        // is_active()==falseのためevaluate_ai_monthly_recruitmentの候補集合から
        // 除外されることを、実際の公開エントリーポイント経由で確認する。
        evaluate_ai_monthly_recruitment(
            &mut h.countries,
            &h.military,
            &h.states,
            &h.buildings,
            &h.war,
            &h.game_date,
            &mut h.country_ai,
            None,
        );
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .recruitment_queue
                .len(),
            0,
            "an eliminated country must never be a recruitment candidate"
        );
    }

    #[test]
    fn an_unresolvable_recruitment_queue_is_caught_by_the_queue_not_empty_check_first() {
        // compute_manpower_ledgerがErrを返す唯一の原因は「募兵キューが未解決の
        // DivisionDefinitionIdを参照している」ことであり、それは必然的に
        // recruitment_queueが非空であることを意味する。AI側は帳簿検証より先に
        // 「キューが空であること」を必須条件としているため(§4)、
        // SkippedLedgerInvalid(未解決定義由来)にAI経路単独では到達できない
        // ——SkippedQueueNotEmptyが先に成立する。帳簿不整合のもう一方の原因
        // (`L>B`、キュー非依存)は`a_ledger_with_living_exceeding_burden_skips_
        // the_country`が独立に検証しており、そちらはこの制約を受けない。
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        h.countries
            .get_mut(CountryId(1))
            .unwrap()
            .recruitment_queue
            .push(RecruitmentQueueItem {
                division_id: DivisionDefinitionId(999),
                target_state: StateId(1),
                days_remaining: 10,
                total_days: 30,
            });
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedQueueNotEmpty
        );
    }

    #[test]
    fn a_ledger_with_living_exceeding_burden_skips_the_country() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)], // B=0
            vec![test_division(0, 1, 5_000, DivisionStatus::Idle)], // L=5,000 > B
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedLedgerInvalid
        );
    }

    #[test]
    fn missing_infantry_definition_skips_recruitment() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        h.military
            .definitions
            .remove(&STANDARD_INFANTRY_DEFINITION_ID);
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedMissingDefinition
        );
    }

    #[test]
    fn no_controlled_state_skips_recruitment() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![state(1, 1, Some(2), 200_000)], // 自国所有だが敵支配
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::SkippedNoControlledState
        );
    }

    /// P25-003: `required_equipment`はもはやtreasuryから差し引かれないため、AI経由の
    /// 募兵成功でもtreasuryは変化しない。
    #[test]
    fn successful_recruitment_reduces_available_and_increases_burden_but_not_treasury() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 5_000, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        assert_eq!(
            decide(&mut h, CountryId(1)),
            AiRecruitmentOutcome::Recruited
        );
        let country = h.countries.get(CountryId(1)).unwrap();
        assert_eq!(country.available_manpower, 1_000_000 - 10_000);
        assert_eq!(country.mobilized_manpower, 5_000 + 10_000);
        assert_eq!(country.treasury, 100_000.0);
        assert_eq!(country.recruitment_queue.len(), 1);
    }

    #[test]
    fn successful_recruitment_leaves_permanent_losses_unchanged() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 13_000, 100_000.0)], // B=13,000
            vec![test_division(0, 1, 10_000, DivisionStatus::Idle)], // L=10,000, K=3,000
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        let ledger_before =
            compute_manpower_ledger(h.countries.get(CountryId(1)).unwrap(), &h.military).unwrap();
        assert_eq!(ledger_before.permanent_military_losses, 3_000);
        decide(&mut h, CountryId(1));
        let ledger_after =
            compute_manpower_ledger(h.countries.get(CountryId(1)).unwrap(), &h.military).unwrap();
        assert_eq!(ledger_after.permanent_military_losses, 3_000);
    }

    #[test]
    fn one_countrys_failure_does_not_prevent_another_countrys_recruitment() {
        let mut h = harness(
            vec![
                test_country(1, 100, 0, 100_000.0), // 予備不足で失敗する
                test_country(2, 1_000_000, 0, 100_000.0),
            ],
            vec![],
            vec![
                owned_state_with_population(1, 1, 200_000),
                owned_state_with_population(2, 2, 200_000),
            ],
            10_000,
        );
        evaluate_ai_monthly_recruitment(
            &mut h.countries,
            &h.military,
            &h.states,
            &h.buildings,
            &h.war,
            &h.game_date,
            &mut h.country_ai,
            None,
        );
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .recruitment_queue
                .len(),
            0
        );
        assert_eq!(
            h.countries
                .get(CountryId(2))
                .unwrap()
                .recruitment_queue
                .len(),
            1
        );
    }

    #[test]
    fn the_player_country_is_never_a_recruitment_candidate() {
        let mut h = harness(
            vec![test_country(1, 1_000_000, 0, 100_000.0)],
            vec![],
            vec![owned_state_with_population(1, 1, 200_000)],
            10_000,
        );
        evaluate_ai_monthly_recruitment(
            &mut h.countries,
            &h.military,
            &h.states,
            &h.buildings,
            &h.war,
            &h.game_date,
            &mut h.country_ai,
            Some(CountryId(1)),
        );
        assert_eq!(
            h.countries
                .get(CountryId(1))
                .unwrap()
                .recruitment_queue
                .len(),
            0,
            "the player's own country must never be auto-recruited for"
        );
    }
}
