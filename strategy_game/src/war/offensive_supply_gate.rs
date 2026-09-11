//! P26-008-B: MilitarySuppliesを考慮したAI攻勢の開始・継続ゲート。
//!
//! P26-008-Aが確立した国家備蓄ヒステリシス(`military::recruitment_supply_gate`)を、
//! 前線ごとのAI作戦判断(`war::military_ai::evaluate_military_ai_decision`)へ
//! **追加条件として**接続する。既存の戦力比110%/90%ヒステリシス・宣戦布告当日の
//! 保護・目標選定・AI評価cadenceのいずれも変更しない。
//!
//! 指標は2系統(P26-008-Aと同じ分離方針):
//! * **国家備蓄健全性**(`national_reserve_days`) — `military::recruitment_supply_gate`
//!   の`has_reserve_for_days`/`stop_days_for`/`resume_days_for`/
//!   `update_recruitment_supply_gate`をそのまま再利用する。攻勢ゲートには
//!   bridge予測(次回月次生産までの橋渡し)という概念自体が存在しないため、
//!   `update_recruitment_supply_gate`へは`bridge_deficit_units=0`(=常に
//!   「bridge不足なし」)を渡す形で再利用する――ヒステリシス状態機械そのものを
//!   複製しない、意図的な設計判断(spec: 既存APIが再利用できない場合だけ抽出せよ、
//!   という指示に対し、この関数はbridge項を無効化するだけで完全に再利用できる)。
//! * **前線配送健全性**(`frontline_min_fulfillment`) — `military::
//!   recruitment_supply_gate::min_fulfillment_over_division_ids`(本タスクで
//!   P26-008-Aの実装から抽出した共通ヘルパー、既存20テストは無変更で全通過)を
//!   前線へ割り当てられたDivisionだけに絞って呼ぶ。
//!
//! **真のヒステリシスの適用範囲(spec §7・§8、重要)**: 永続化する`offensive_
//! supply_blocked`(国家備蓄ヒステリシスの結果)は**新規攻勢の開始可否だけ**を
//! 決める。既存の`Offensive`を**継続**できるかどうかは、この永続フラグを一切
//! 参照せず、その評価時点の`frontline_min_fulfillment`を直接判定する
//! (`is_frontline_delivery_healthy`)。これにより「国家備蓄が一時的にstop水準を
//! 下回っただけでは進行中の攻勢を止めない」という要求(spec §7)と、「新規攻勢は
//! resumeしきい値に達するまで再開しない」という真のヒステリシス(spec §8)を、
//! 矛盾なく両立させる。

use crate::common::DivisionId;
use crate::country::CountryData;
use crate::economy::resources::{
    MilitarySuppliesUnitsError, ResourceType, military_supplies_value_to_units,
};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::military::recruitment_supply_gate::{
    min_fulfillment_over_division_ids, update_recruitment_supply_gate,
};
use crate::military::supply_initialization::{SupplyDemandError, country_daily_demand_units};
use crate::war::data::WarRegistry;

/// 前線配送健全性のしきい値(spec §5.3)。CountryId・戦争・前線・国家Tierに
/// よって変化させない、単一の正規値。
pub const MIN_OFFENSIVE_SUPPLY_FULFILLMENT: f64 = 0.34;

/// `evaluate_offensive_supply_status`が失敗し得る理由(いずれも防御的な分岐、
/// 通常の正常なゲーム状態では到達しない)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffensiveSupplyEvaluationError {
    Demand(SupplyDemandError),
    /// 現在備蓄がNaN・負数等で正規unitsへ変換できない。
    InvalidCurrentStockpile,
}

impl From<SupplyDemandError> for OffensiveSupplyEvaluationError {
    fn from(e: SupplyDemandError) -> Self {
        OffensiveSupplyEvaluationError::Demand(e)
    }
}

/// 1回分の前線補給ゲート評価結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffensiveSupplyEvaluation {
    /// 更新後の「新規攻勢の開始待ち(補給回復待ち)」状態。
    /// `FrontlinePlan.offensive_supply_blocked`へそのまま書き戻す。
    pub blocked_after: bool,
    /// この評価時点で前線配送が健全か("対象Divisionなし"は健全扱い、spec §5.2)。
    /// 攻勢の**継続**可否はこの値を直接参照し、`blocked_after`は参照しない。
    pub delivery_fully_healthy: bool,
}

/// 前線へ割り当てられた生存Divisionのうち、正の日次需要を持つものだけを対象に、
/// `MilitaryRegistry::supply_fulfillment_of`の最小値を求める(spec §5.2)。
///
/// 対象外(spec §5.2の否定条件そのもの): 既に消滅したDivision、需要0の
/// Division。`assigned_division_ids`は呼び出し時点で別Frontlineのものは
/// 含まれない(`FrontlineRegistry::assign_division`が1 Division=1 Frontlineを
/// 保証する既存契約)ため、ここでは生存判定と需要0判定だけを行う。
/// 対象Divisionが1件もなければ`None`(「配送不足signalなし」)。
pub fn frontline_min_fulfillment(
    assigned_division_ids: &[DivisionId],
    military_registry: &MilitaryRegistry,
) -> Result<Option<f64>, SupplyDemandError> {
    let mut ids: Vec<DivisionId> = assigned_division_ids
        .iter()
        .copied()
        .filter(|id| {
            military_registry
                .divisions
                .get(id)
                .map(|d| d.manpower > 0 && d.status != DivisionStatus::Destroyed)
                .unwrap_or(false)
        })
        .collect();
    ids.sort_by_key(|id| id.0);
    min_fulfillment_over_division_ids(&ids, military_registry)
}

/// 前線配送が「攻勢を開始・継続してよい」水準にあるか判定する(spec §5.3)。
/// 対象Divisionなし(`None`)は「配送不足signalなし」として健全扱いにする
/// (spec §5.2・P26-008-Aの`is_delivery_fully_healthy`と同じ設計方針)。
pub fn is_frontline_delivery_healthy(min_fulfillment: Option<f64>) -> bool {
    match min_fulfillment {
        Some(f) => f >= MIN_OFFENSIVE_SUPPLY_FULFILLMENT,
        None => true,
    }
}

/// 1つの(前線, 指揮国)ペアについて、国家備蓄ヒステリシスと前線配送健全性を
/// まとめて評価する。`treasury`・`recruitment_queue`・Division・
/// `available_manpower`・`MilitarySupplies`備蓄のいずれも変更しない
/// (既存の`military::recruitment_supply_gate::evaluate_recruitment_supply_gate`
/// と同じ契約)。
pub fn evaluate_offensive_supply_status(
    country: &CountryData,
    war_registry: &WarRegistry,
    military_registry: &MilitaryRegistry,
    assigned_division_ids: &[DivisionId],
    currently_blocked: bool,
) -> Result<OffensiveSupplyEvaluation, OffensiveSupplyEvaluationError> {
    let is_at_war = war_registry.is_country_at_war(country.id);

    let current_stockpile_value = country.stockpile.get(ResourceType::MilitarySupplies);
    let current_stockpile_units = military_supplies_value_to_units(current_stockpile_value)
        .map_err(|_: MilitarySuppliesUnitsError| {
            OffensiveSupplyEvaluationError::InvalidCurrentStockpile
        })? as u128;

    let current_total_demand_units = country_daily_demand_units(
        country.id,
        &military_registry.divisions,
        &military_registry.definitions,
    )?;

    let min_fulfillment = frontline_min_fulfillment(assigned_division_ids, military_registry)?;
    let delivery_fully_healthy = is_frontline_delivery_healthy(min_fulfillment);

    // P26-008-B: 攻勢ゲートにbridge予測は適用しない(モジュールdoc参照)ため、
    // bridge_deficit_units=0(常に「bridge不足なし」)を渡して既存の正規
    // ヒステリシス状態機械をそのまま再利用する。
    let blocked_after = update_recruitment_supply_gate(
        currently_blocked,
        is_at_war,
        current_stockpile_units,
        current_total_demand_units,
        0,
        delivery_fully_healthy,
    );

    Ok(OffensiveSupplyEvaluation {
        blocked_after,
        delivery_fully_healthy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, StateId};
    use crate::military::data::{Division, DivisionDefinition, DivisionSize, DivisionType};
    use crate::war::data::{War, WarStatus};

    const CID: CountryId = CountryId(0);
    const DEF_ID: DivisionDefinitionId = DivisionDefinitionId(1);

    fn infantry_def() -> DivisionDefinition {
        DivisionDefinition {
            id: DEF_ID,
            name: "Standard Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
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

    fn living_division(id: usize, manpower: u64) -> Division {
        Division {
            id: DivisionId(id),
            owner: CID,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: manpower.max(1),
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 50.0,
            max_morale: 50.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DEF_ID,
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn registry_with(divisions: Vec<Division>) -> MilitaryRegistry {
        let mut reg = MilitaryRegistry::default();
        reg.definitions.insert(DEF_ID, infantry_def());
        for d in divisions {
            reg.divisions.insert(d.id, d);
        }
        reg
    }

    // ── frontline_min_fulfillment: 対象集合(spec §5.2) ─────────────────────

    #[test]
    fn empty_assignment_means_no_delivery_signal() {
        let registry = registry_with(vec![]);
        let result = frontline_min_fulfillment(&[], &registry).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn zero_demand_division_is_excluded_from_the_minimum() {
        let mut registry = registry_with(vec![living_division(1, 0)]); // manpower=0除外
        registry.division_supply_fulfillment.insert(DivisionId(1), 0.0);
        let result = frontline_min_fulfillment(&[DivisionId(1)], &registry).unwrap();
        assert_eq!(result, None, "需要0のDivisionは母集合から除外されるはず");
    }

    #[test]
    fn destroyed_division_is_excluded() {
        let mut d = living_division(1, 10_000);
        d.status = DivisionStatus::Destroyed;
        let mut registry = registry_with(vec![d]);
        registry.division_supply_fulfillment.insert(DivisionId(1), 0.0);
        let result = frontline_min_fulfillment(&[DivisionId(1)], &registry).unwrap();
        assert_eq!(result, None, "消滅Divisionは対象外のはず");
    }

    #[test]
    fn only_assigned_ids_are_considered_not_the_whole_registry() {
        // registryには2個Divisionがあるが、前線へは1個だけ割り当てられている。
        let mut registry = registry_with(vec![living_division(1, 10_000), living_division(2, 10_000)]);
        registry.division_supply_fulfillment.insert(DivisionId(1), 1.0);
        registry.division_supply_fulfillment.insert(DivisionId(2), 0.0); // 別前線相当、無視されるはず
        let result = frontline_min_fulfillment(&[DivisionId(1)], &registry).unwrap();
        assert_eq!(result, Some(1.0));
    }

    // ── 0.34境界(spec §13-A) ────────────────────────────────────────────────

    #[test]
    fn boundary_just_below_0_34_is_unhealthy() {
        assert!(!is_frontline_delivery_healthy(Some(0.339_999)));
    }

    #[test]
    fn boundary_exactly_0_34_is_healthy() {
        assert!(is_frontline_delivery_healthy(Some(0.34)));
    }

    #[test]
    fn fully_supplied_is_healthy() {
        assert!(is_frontline_delivery_healthy(Some(1.0)));
    }

    #[test]
    fn no_signal_is_healthy() {
        assert!(is_frontline_delivery_healthy(None));
    }

    // ── evaluate_offensive_supply_status: 統合 ──────────────────────────────

    fn country_with_supplies(supplies: f64) -> CountryData {
        let mut c = CountryData { id: CID, ..CountryData::default() };
        c.stockpile.set(ResourceType::MilitarySupplies, supplies);
        c
    }

    fn war_registry_at_war(country: CountryId) -> WarRegistry {
        let mut reg = WarRegistry::default();
        reg.wars.insert(
            crate::common::WarId(0),
            War {
                id: crate::common::WarId(0),
                name: "Test".to_string(),
                attackers: [country].into_iter().collect(),
                defenders: [CountryId(99)].into_iter().collect(),
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
                occupied_states: std::collections::HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: std::collections::HashSet::new(),
            },
        );
        reg
    }

    #[test]
    fn ample_reserve_and_healthy_delivery_stays_unblocked() {
        let registry = registry_with(vec![living_division(1, 10_000)]);
        let country = country_with_supplies(1000.0);
        let war_registry = war_registry_at_war(CID);
        let eval = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(!eval.blocked_after);
        assert!(eval.delivery_fully_healthy);
    }

    /// 生産経路では`evaluate_military_ai_decision`が常にactive warの前線に対して
    /// しか呼ばれないため、`is_at_war==false`(平時のstop=20/resume=35)の分岐は
    /// 実際の呼び出し経路からは到達しないが、この純粋関数自体は汎用的にis_at_warを
    /// 受け取る設計であるため、平時分岐も独立して検証する(spec §6の20/35日表)。
    #[test]
    fn peacetime_stop_and_resume_thresholds_are_used_when_not_at_war() {
        let registry = registry_with(vec![living_division(1, 10_000)]);
        let war_registry = WarRegistry::default(); // 参戦なし

        // 平時stop=20日: 19日は不足、20日は充足。
        let below_stop = country_with_supplies(19.0);
        let eval = evaluate_offensive_supply_status(
            &below_stop,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(eval.blocked_after, "平時stop(20日)未満はblockedになるはず");

        let at_stop = country_with_supplies(20.0);
        let eval2 = evaluate_offensive_supply_status(
            &at_stop,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(!eval2.blocked_after, "平時stop(20日)ちょうどはblockedにならないはず");

        // 平時resume=35日: 34日はblocked維持、35日で解除。
        let below_resume = country_with_supplies(34.0);
        let eval3 = evaluate_offensive_supply_status(
            &below_resume,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(eval3.blocked_after, "平時resume(35日)未満はblocked維持のはず");

        let at_resume = country_with_supplies(35.0);
        let eval4 = evaluate_offensive_supply_status(
            &at_resume,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(!eval4.blocked_after, "平時resume(35日)ちょうどでblocked解除されるはず");
    }

    #[test]
    fn low_reserve_below_wartime_stop_blocks_future_starts() {
        // 戦時stop=40日、需要1.0/日 -> 40未満なら不足
        let registry = registry_with(vec![living_division(1, 10_000)]);
        let country = country_with_supplies(39.0);
        let war_registry = war_registry_at_war(CID);
        let eval = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(eval.blocked_after);
    }

    #[test]
    fn unhealthy_delivery_alone_blocks_even_with_ample_reserve() {
        let mut registry = registry_with(vec![living_division(1, 10_000)]);
        registry.division_supply_fulfillment.insert(DivisionId(1), 0.1);
        let country = country_with_supplies(100_000.0);
        let war_registry = war_registry_at_war(CID);
        let eval = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(eval.blocked_after);
        assert!(!eval.delivery_fully_healthy);
    }

    #[test]
    fn middle_band_preserves_prior_blocked_state() {
        // 戦時: stop=40, resume=70。45日は中間帯。
        let registry = registry_with(vec![living_division(1, 10_000)]);
        let country = country_with_supplies(45.0);
        let war_registry = war_registry_at_war(CID);

        let was_unblocked = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            false,
        )
        .unwrap();
        assert!(!was_unblocked.blocked_after, "中間帯かつ以前unblockedなら維持されるはず");

        let was_blocked = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(was_blocked.blocked_after, "中間帯かつ以前blockedなら維持されるはず");
    }

    #[test]
    fn resume_threshold_clears_the_blocked_state() {
        let registry = registry_with(vec![living_division(1, 10_000)]);
        let war_registry = war_registry_at_war(CID);

        let below_resume = country_with_supplies(69.0);
        let eval = evaluate_offensive_supply_status(
            &below_resume,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(eval.blocked_after, "戦時resume(70日)未満はblocked維持のはず");

        let at_resume = country_with_supplies(70.0);
        let eval2 = evaluate_offensive_supply_status(
            &at_resume,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(!eval2.blocked_after, "戦時resume(70日)ちょうどでblocked解除されるはず");
    }

    #[test]
    fn resume_reserve_alone_does_not_clear_blocked_if_delivery_unhealthy() {
        let mut registry = registry_with(vec![living_division(1, 10_000)]);
        registry.division_supply_fulfillment.insert(DivisionId(1), 0.0);
        let country = country_with_supplies(1_000_000.0); // reserveは十分すぎるほど
        let war_registry = war_registry_at_war(CID);
        let eval = evaluate_offensive_supply_status(
            &country,
            &war_registry,
            &registry,
            &[DivisionId(1)],
            true,
        )
        .unwrap();
        assert!(
            eval.blocked_after,
            "配送率0.34未満ならreserveが十分でもblocked解除しないはず"
        );
    }

    #[test]
    fn zero_demand_country_never_blocks_on_reserve_alone() {
        // Divisionなし(現在需要0) -> reserve判定は常にtrue、配送もNone(健全扱い)。
        let registry = registry_with(vec![]);
        let country = country_with_supplies(0.0);
        let war_registry = war_registry_at_war(CID);
        let eval = evaluate_offensive_supply_status(&country, &war_registry, &registry, &[], false)
            .unwrap();
        assert!(!eval.blocked_after);
        assert!(eval.delivery_fully_healthy);
    }
}
