use crate::app::time::DayChangedMessage;
use crate::common::{CountryId, DivisionId, FrontlineId, StateId};
use crate::country::{CountryData, CountryRegistry, PlayerCountry};
use crate::military::data::{Division, DivisionStatus, MilitaryRegistry};
use crate::military::pathfinding::find_path;
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};
use crate::war::frontline::{
    Frontline, FrontlinePlan, FrontlineRegistry, FrontlineStance, determine_offensive_objective,
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 軍事AIの判断理由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MilitaryAiDecisionReason {
    #[default]
    NoActiveWar,
    NoValidFrontline,
    NoAvailableDivision,
    NoReachableFrontline,
    NoValidTarget,
    Recovering,
    EnemyStronger,
    ComparableStrength,
    SufficientAdvantage,
    TargetCaptured,
    /// P26-008-B: MilitarySupplies補給不足(国家備蓄の停止水準未達、または前線
    /// 配送率が0.34未満)により、戦力比条件を満たしていても攻勢を開始・継続
    /// しなかった(既存の`ComparableStrength`/`EnemyStronger`へ偽装しない、
    /// P26-008-Aの`SkippedSupplyShortage`と同じ設計判断)。
    SupplyShortage,
}

impl MilitaryAiDecisionReason {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            MilitaryAiDecisionReason::NoActiveWar => "military_ai_reason.no_active_war",
            MilitaryAiDecisionReason::NoValidFrontline => "military_ai_reason.no_valid_frontline",
            MilitaryAiDecisionReason::NoAvailableDivision => {
                "military_ai_reason.no_available_division"
            }
            MilitaryAiDecisionReason::NoReachableFrontline => {
                "military_ai_reason.no_reachable_frontline"
            }
            MilitaryAiDecisionReason::NoValidTarget => "military_ai_reason.no_valid_target",
            MilitaryAiDecisionReason::Recovering => "military_ai_reason.recovering",
            MilitaryAiDecisionReason::EnemyStronger => "military_ai_reason.enemy_stronger",
            MilitaryAiDecisionReason::ComparableStrength => {
                "military_ai_reason.comparable_strength"
            }
            MilitaryAiDecisionReason::SufficientAdvantage => {
                "military_ai_reason.sufficient_advantage"
            }
            MilitaryAiDecisionReason::TargetCaptured => "military_ai_reason.target_captured",
            MilitaryAiDecisionReason::SupplyShortage => "military_ai_reason.supply_shortage",
        }
    }
}

/// AI国家ごとの作戦状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryAiState {
    pub country_id: CountryId,
    pub last_evaluated_day: u32,
    pub last_decision_reason: MilitaryAiDecisionReason,
    pub estimated_own_power: u64,
    pub estimated_enemy_power: u64,
    pub dirty: bool,
}

impl MilitaryAiState {
    pub fn new(country_id: CountryId) -> Self {
        Self {
            country_id,
            last_evaluated_day: 0,
            last_decision_reason: MilitaryAiDecisionReason::NoActiveWar,
            estimated_own_power: 0,
            estimated_enemy_power: 0,
            dirty: true,
        }
    }
}

/// 軍事AIを集中管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryAiRegistry {
    pub ai_states: HashMap<CountryId, MilitaryAiState>,
    pub dirty: bool,
}

impl MilitaryAiRegistry {
    /// 指定国家のAI状態を取得または作成
    pub fn get_or_create_mut(&mut self, country_id: CountryId) -> &mut MilitaryAiState {
        self.ai_states
            .entry(country_id)
            .or_insert_with(|| MilitaryAiState::new(country_id))
    }

    /// 全AI状態を dirty にする
    pub fn mark_all_dirty(&mut self) {
        for state in self.ai_states.values_mut() {
            state.dirty = true;
        }
        self.dirty = true;
    }

    /// 特定国家のAI状態を dirty にする
    pub fn mark_country_dirty(&mut self, country_id: CountryId) {
        if let Some(state) = self.ai_states.get_mut(&country_id) {
            state.dirty = true;
        }
        self.dirty = true;
    }
}

/// 単一陸軍の有効戦力計算 (manpower * org / max_org)
pub fn evaluate_division_power(division: &Division) -> u64 {
    if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
        return 0;
    }
    if division.max_organization <= 0.0 {
        return division.manpower;
    }
    let org_ratio = (division.organization / division.max_organization).clamp(0.0, 1.0);
    ((division.manpower as f64) * (org_ratio as f64)) as u64
}

/// 部隊がAI割当対象として使用可能か検証する
pub fn is_division_available_for_ai(
    division: &Division,
    commanding_country: CountryId,
    frontline_registry: &FrontlineRegistry,
) -> bool {
    if division.owner != commanding_country {
        return false;
    }
    if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
        return false;
    }
    // 戦闘中・撤退中の部隊は新規割当対象にしない（割当済み前線への所属維持は可）
    if division.status == DivisionStatus::Fighting || division.status == DivisionStatus::Retreating
    {
        return false;
    }
    // 既に別の前線に所属している場合は対象外
    if frontline_registry
        .division_frontline_map
        .contains_key(&division.id)
    {
        return false;
    }
    true
}

/// 攻勢目標候補の決定的な選択（首都・グラフ距離考慮）
pub fn select_ai_offensive_objective(
    war: &War,
    commanding_country: CountryId,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
    frontline: &Frontline,
) -> Option<StateId> {
    // まず Phase 17 の標準目標選択（戦争目標、奪回領土）を試す
    if let Some(primary_obj) =
        determine_offensive_objective(war, commanding_country, state_registry)
        && let Some(s) = state_registry.get(primary_obj)
        && !s.is_sea
        && s.controller() != commanding_country
    {
        return Some(primary_obj);
    }

    let enemy_id = if commanding_country == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    // 経路探索は自国だけでなく自陣営全体(多国間戦争の同盟国)を通過許可国に含める
    // (プレイヤー報告により発見: 単一命令国家だけを許可すると、増援ルートが同盟国領を
    // 経由する場合に誤って「経路なし」となり、無関係なDivisionまで動けなくなっていた)。
    let (own_side, enemy_side) = {
        let own = war.own_side_of(commanding_country);
        let enemy = war.opponents_of(commanding_country);
        if own.is_empty() || enemy.is_empty() {
            (vec![commanding_country], vec![enemy_id])
        } else {
            (own, enemy)
        }
    };

    let front_regions = if commanding_country == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    if front_regions.is_empty() {
        return None;
    }

    // 候補1: 敵国の首都（到達可能かつ未支配の場合）
    if let Some(enemy_country) = country_registry.get(enemy_id) {
        let cap_id = enemy_country.capital_state_id;
        if let Some(cap_state) = state_registry.get(cap_id)
            && !cap_state.is_sea
            && cap_state.controller() == enemy_id
        {
            let reachable = front_regions
                .iter()
                .any(|&fr| find_path(fr, cap_id, state_registry, &own_side, &enemy_side).is_some());
            if reachable {
                return Some(cap_id);
            }
        }
    }

    // 候補2: 前線からの最短グラフ距離にある敵国支配地域（StateId昇順で確定）
    let mut enemy_states: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.controller() == enemy_id)
        .map(|s| s.id)
        .collect();
    enemy_states.sort_by_key(|s| s.0);

    let mut candidates_with_dist: Vec<(StateId, usize)> = Vec::new();

    for &enemy_state_id in &enemy_states {
        let min_dist = front_regions
            .iter()
            .filter_map(|&fr| {
                find_path(fr, enemy_state_id, state_registry, &own_side, &enemy_side)
                    .map(|p| p.len())
            })
            .min();

        if let Some(dist) = min_dist {
            candidates_with_dist.push((enemy_state_id, dist));
        }
    }

    candidates_with_dist.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)));

    candidates_with_dist.first().map(|(sid, _)| *sid)
}

/// 自軍割当戦力と敵軍保有戦力の集計・評価
pub fn evaluate_powers(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    commanding_country: CountryId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> (u64, u64) {
    let enemy_id = if commanding_country == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    // 自軍割り当て部隊の有効戦力合計
    let own_power: u64 = plan
        .assigned_division_ids
        .iter()
        .filter_map(|&id| military_registry.divisions.get(&id))
        .filter(|a| a.owner == commanding_country && a.status != DivisionStatus::Destroyed)
        .map(evaluate_division_power)
        .sum();

    // 敵国の全戦闘可能部隊の有効戦力合計
    let enemy_power: u64 = military_registry
        .divisions
        .values()
        .filter(|a| {
            a.owner == enemy_id
                && a.manpower > 0
                && a.status != DivisionStatus::Destroyed
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(evaluate_division_power)
        .sum();

    (own_power, enemy_power)
}

/// P28-003: 攻勢可否の前線配送判定に使うDivision集合を求める純粋関数。
///
/// `assigned`(前線計画の割当Division)から、**飛び地に取り残されたDivision**だけを除く。
/// 取り残されたDivisionとは、現在州の`controller`が指揮国自身であり、かつその州が
/// `capital_connected`(指揮国の首都から自国支配州をたどる陸路連結集合、
/// `military::capital_connectivity::connected_states_from_capital`の結果)に含まれない
/// Divisionである。こうしたDivisionは日次補給の配分対象外で補給充足率が恒久的に0になり、
/// 国家備蓄も消費しないため、前線への配送の健全性を測る最小値から外す。
///
/// 除かないもの:
/// * 敵支配州(controllerが指揮国以外)にいるDivision——攻撃中の部隊で、P27-005の一歩先の
///   前線補給の対象になり得るため、従来どおり配送判定に含める。
/// * 連結済みの自国支配州にいるDivision、登録に存在しないDivision、州が存在しないDivision。
///
/// 除いた結果が空になる場合(割当が取り残されたDivisionだけの場合)は、`assigned`を
/// そのまま返す。補給不能な部隊しかいない前線を、空集合(「配送不足signalなし」)として
/// 攻勢許可しないため。`assigned`が元々空なら空を返す(既存の割当0件の扱いのまま)。
/// 返り値は`assigned`の順序を保つ(最小値の計算は順序に依存しない)。
pub fn offensive_delivery_division_ids(
    assigned: &[DivisionId],
    commanding_country: CountryId,
    capital_connected: &std::collections::HashSet<StateId>,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> Vec<DivisionId> {
    let is_stranded = |id: &DivisionId| {
        military_registry.divisions.get(id).is_some_and(|division| {
            state_registry.get(division.current_state).is_some_and(|state| {
                state.controller() == commanding_country && !capital_connected.contains(&state.id)
            })
        })
    };
    let scoped: Vec<DivisionId> = assigned.iter().copied().filter(|id| !is_stranded(id)).collect();
    if scoped.is_empty() { assigned.to_vec() } else { scoped }
}

/// 前線に対するAI命令スタンスと理由の決定（ヒステリシス考慮）。
///
/// P26-008-B: 戻り値の第3要素は、更新後の`FrontlinePlan.offensive_supply_blocked`
/// (新規攻勢の開始待ち状態、真のヒステリシス)。呼び出し側がPlanへ書き戻す責務を
/// 持つ(この関数自体は`plan`を変更しない、既存の純粋関数という契約を維持)。
/// 早期returnの理由に関わらず毎回この値を計算・返却する(spec §8.2: 現在の
/// スタンスに関わらず、国家備蓄・前線配送の観測自体は常に行う)。
///
/// P28-003: `delivery_division_ids`は前線配送判定(`frontline_min_fulfillment`の
/// 最小値を取る母集合)だけに使うDivision集合で、通常は
/// `offensive_delivery_division_ids`で`plan.assigned_division_ids`から飛び地に取り残された
/// Divisionを除いたもの。割当0件の判定・即応可能Divisionの判定・戦力比(`own_power`/
/// `enemy_power`)には使わず、これらは従来どおり`plan.assigned_division_ids`で判定する。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_military_ai_decision(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    delivery_division_ids: &[DivisionId],
    own_power: u64,
    enemy_power: u64,
    has_objective: bool,
    military_registry: &MilitaryRegistry,
    country: &CountryData,
    war_registry: &WarRegistry,
) -> (FrontlineStance, MilitaryAiDecisionReason, bool) {
    // P26-008-B: 補給観測。評価自体が失敗する防御的なケース(通常到達しない)では、
    // 既存のblocked状態を変えず、配送は健全とみなして既存ロジックへフォールバックする
    // (安全側は「新規攻勢を増やさない」であって「進行中攻勢を無条件停止する」ではない
    // ため、delivery_fully_healthy=trueをフォールバック値にする)。
    let (offensive_supply_blocked, delivery_fully_healthy) =
        match crate::war::offensive_supply_gate::evaluate_offensive_supply_status(
            country,
            war_registry,
            military_registry,
            delivery_division_ids,
            plan.offensive_supply_blocked,
        ) {
            Ok(eval) => (eval.blocked_after, eval.delivery_fully_healthy),
            Err(_) => {
                debug!(
                    "[MilitaryAi] country {:?}: offensive supply gate evaluation failed, \
                     keeping prior blocked state and treating delivery as healthy this evaluation",
                    country.id
                );
                (plan.offensive_supply_blocked, true)
            }
        };

    if frontline.border_region_pairs.is_empty() {
        return (
            FrontlineStance::Stopped,
            MilitaryAiDecisionReason::NoValidFrontline,
            offensive_supply_blocked,
        );
    }

    if plan.assigned_division_ids.is_empty() {
        return (
            FrontlineStance::Stopped,
            MilitaryAiDecisionReason::NoAvailableDivision,
            offensive_supply_blocked,
        );
    }

    // 即応可能な待機・移動中陸軍があるか確認
    let has_ready_divisions = plan.assigned_division_ids.iter().any(|&id| {
        military_registry
            .divisions
            .get(&id)
            .map(|a| a.status == DivisionStatus::Idle || a.status == DivisionStatus::Moving)
            .unwrap_or(false)
    });

    if !has_ready_divisions {
        return (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::Recovering,
            offensive_supply_blocked,
        );
    }

    if !has_objective {
        return (
            FrontlineStance::Defend,
            MilitaryAiDecisionReason::NoValidTarget,
            offensive_supply_blocked,
        );
    }

    // 攻勢条件: 110% (own * 1000 >= enemy * 1100)
    // 攻勢維持条件: 90% (own * 1000 >= enemy * 900)
    let current_stance = plan.stance;

    let can_start_offensive = if enemy_power == 0 {
        own_power > 0
    } else {
        (own_power as u128) * 1000 >= (enemy_power as u128) * 1100
    };

    let can_continue_offensive = if enemy_power == 0 {
        own_power > 0
    } else {
        (own_power as u128) * 1000 >= (enemy_power as u128) * 900
    };

    let (stance, reason) = if current_stance == FrontlineStance::Offensive {
        // P26-008-B spec §7: 進行中の攻勢は、国家備蓄の一時低下(offensive_supply_
        // blocked)だけでは止めない――前線配送健全性(delivery_fully_healthy)を
        // 直接判定する。既存90%条件はそのまま維持する。
        if !delivery_fully_healthy {
            (FrontlineStance::Defend, MilitaryAiDecisionReason::SupplyShortage)
        } else if can_continue_offensive {
            (FrontlineStance::Offensive, MilitaryAiDecisionReason::SufficientAdvantage)
        } else {
            (FrontlineStance::Defend, MilitaryAiDecisionReason::ComparableStrength)
        }
    } else if can_start_offensive {
        // P26-008-B spec §6: 新規攻勢は既存110%条件に加え、補給回復待ちでないこと
        // (offensive_supply_blocked==false、国家備蓄stop以上・前線配送0.34以上の
        // 両方を内包する)を要求する。
        if offensive_supply_blocked {
            (FrontlineStance::Defend, MilitaryAiDecisionReason::SupplyShortage)
        } else {
            (FrontlineStance::Offensive, MilitaryAiDecisionReason::SufficientAdvantage)
        }
    } else if enemy_power > own_power {
        (FrontlineStance::Defend, MilitaryAiDecisionReason::EnemyStronger)
    } else {
        (FrontlineStance::Defend, MilitaryAiDecisionReason::ComparableStrength)
    };

    (stance, reason, offensive_supply_blocked)
}

/// 未割り当ての有効陸軍を評価し、適切な前線へ自動割り当てする
pub fn assign_unassigned_divisions_to_frontlines(
    country_id: CountryId,
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    current_date: Option<&str>,
) {
    // AI国家の未割当・使用可能陸軍を取得 (DivisionId昇順)
    let mut available_division_ids: Vec<DivisionId> = military_registry
        .divisions
        .values()
        .filter(|a| {
            a.owner == country_id
                && a.manpower > 0
                && a.status != DivisionStatus::Destroyed
                && !frontline_registry
                    .division_frontline_map
                    .contains_key(&a.id)
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(|a| a.id)
        .collect();

    available_division_ids.sort_by_key(|a| a.0);

    if available_division_ids.is_empty() {
        return;
    }

    // 進行中戦争の前線を取得 (WarId昇順, FrontlineId昇順)
    let mut target_frontlines: Vec<Frontline> = frontline_registry
        .frontlines
        .values()
        .filter(|fl| {
            if fl.attacker_country_id != country_id && fl.defender_country_id != country_id {
                return false;
            }
            if let Some(curr) = current_date
                && let Some(war) = war_registry.wars.get(&fl.war_id)
                && war.start_date == curr
            {
                return false;
            }
            war_registry
                .wars
                .get(&fl.war_id)
                .map(|w| w.status == WarStatus::Active)
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    target_frontlines.sort_by(|a, b| {
        a.war_id
            .0
            .cmp(&b.war_id.0)
            .then_with(|| a.frontline_id.0.cmp(&b.frontline_id.0))
    });

    if target_frontlines.is_empty() {
        return;
    }

    for division_id in available_division_ids {
        let division = match military_registry.divisions.get(&division_id) {
            Some(a) => a,
            None => continue,
        };

        // 部隊が到達可能な前線を評価して最適前線を選択
        let mut best_fl_id: Option<FrontlineId> = None;

        // 到達可能性と戦力不足度を評価
        // 優先度: 1. 割当0部隊 -> 2. (own * enemy_B < own_B * enemy) 即ち戦力比率最小
        let mut candidates: Vec<(FrontlineId, usize, u64, u64)> = Vec::new();

        for fl in &target_frontlines {
            let enemy_id = if country_id == fl.attacker_country_id {
                fl.defender_country_id
            } else {
                fl.attacker_country_id
            };

            // 経路探索は自陣営全体(多国間戦争の同盟国)を通過許可国に含める
            // (select_ai_offensive_objectiveと同じ理由)。
            let (own_side, enemy_side) = war_registry
                .wars
                .get(&fl.war_id)
                .map(|w| (w.own_side_of(country_id), w.opponents_of(country_id)))
                .filter(|(own, enemy)| !own.is_empty() && !enemy.is_empty())
                .unwrap_or_else(|| (vec![country_id], vec![enemy_id]));

            let front_regions = if country_id == fl.attacker_country_id {
                &fl.attacker_front_regions
            } else {
                &fl.defender_front_regions
            };

            let is_reachable = front_regions.iter().any(|&fr| {
                find_path(
                    division.current_state,
                    fr,
                    state_registry,
                    &own_side,
                    &enemy_side,
                )
                .is_some()
            });

            if is_reachable {
                let plan = frontline_registry.get_plan(fl.frontline_id, country_id);
                let assigned_count = plan.map(|p| p.assigned_division_ids.len()).unwrap_or(0);
                let (own_p, enemy_p) = plan
                    .map(|p| evaluate_powers(fl, p, country_id, military_registry, state_registry))
                    .unwrap_or((0, 0));

                candidates.push((fl.frontline_id, assigned_count, own_p, enemy_p));
            }
        }

        candidates.sort_by(|a, b| {
            // 割当部隊数0優先
            let a_has_zero = a.1 == 0;
            let b_has_zero = b.1 == 0;
            if a_has_zero != b_has_zero {
                return b_has_zero.cmp(&a_has_zero);
            }

            // 比率比較 (a_own * b_enemy vs b_own * a_enemy)
            let a_cross = (a.2 as u128) * ((b.3 + 1) as u128);
            let b_cross = (b.2 as u128) * ((a.3 + 1) as u128);
            a_cross.cmp(&b_cross).then_with(|| a.0.0.cmp(&b.0.0))
        });

        if let Some((fl_id, _, _, _)) = candidates.first() {
            best_fl_id = Some(*fl_id);
        }

        if let Some(target_fl_id) = best_fl_id {
            let _ = frontline_registry.assign_division(
                division_id,
                target_fl_id,
                country_id,
                military_registry,
                war_registry,
            );
        }
    }
}

/// 日次軍事AI処理のエントリーポイント
#[allow(clippy::too_many_arguments)]
pub fn process_daily_military_ai(
    current_day: u32,
    current_date: &str,
    player_country: &PlayerCountry,
    country_registry: &CountryRegistry,
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    ai_registry: &mut MilitaryAiRegistry,
) {
    let player_cid = player_country.0;

    // AI対象国家の抽出 (CountryId昇順)
    let mut ai_country_ids: Vec<CountryId> = country_registry
        .countries
        .iter()
        .map(|c| c.id)
        .filter(|&cid| Some(cid) != player_cid)
        .collect();
    ai_country_ids.sort_by_key(|c| c.0);

    for country_id in ai_country_ids {
        // 進行中の戦争を取得 (WarId昇順)
        let mut active_wars: Vec<&War> = war_registry
            .wars
            .values()
            .filter(|w| {
                w.status == WarStatus::Active
                    && (w.attackers.contains(&country_id) || w.defenders.contains(&country_id))
            })
            .collect();
        active_wars.sort_by_key(|w| w.id.0);

        if active_wars.is_empty() {
            let ai_state = ai_registry.get_or_create_mut(country_id);
            ai_state.last_evaluated_day = current_day;
            ai_state.last_decision_reason = MilitaryAiDecisionReason::NoActiveWar;
            ai_state.estimated_own_power = 0;
            ai_state.estimated_enemy_power = 0;
            ai_state.dirty = false;
            continue;
        }

        let ai_state = ai_registry.get_or_create_mut(country_id);
        let is_evaluation_due =
            current_day.saturating_sub(ai_state.last_evaluated_day) >= 3 || ai_state.dirty;

        if !is_evaluation_due {
            continue;
        }

        // 1. 未割当陸軍の複数前線分配 (宣戦布告当日の前線は対象外)
        assign_unassigned_divisions_to_frontlines(
            country_id,
            war_registry,
            state_registry,
            military_registry,
            frontline_registry,
            Some(current_date),
        );

        let mut total_own_power: u64 = 0;
        let mut total_enemy_power: u64 = 0;
        let mut last_reason = MilitaryAiDecisionReason::NoValidFrontline;
        let mut skipped_due_to_start_date = false;

        // P28-003: 前線配送判定の母集合から飛び地のDivisionを除くための首都連結集合。
        // 評価の日の州controllerから毎回計算し直す(日次補給`military::daily_supply`と同じ
        // `connected_states_from_capital`を再利用、永続化しない)。首都がない場合は空集合。
        let capital_connected = country_registry
            .get(country_id)
            .and_then(|c| c.current_capital_state_id())
            .map(|capital| {
                crate::military::capital_connectivity::connected_states_from_capital(
                    capital,
                    country_id,
                    state_registry,
                )
            })
            .unwrap_or_default();

        for war in &active_wars {
            // 宣戦布告当日は軍事AIの移動命令を発行しない（プレイヤー保護と不意打ち防止）
            if war.start_date == current_date {
                skipped_due_to_start_date = true;
                continue;
            }

            if let Some(fl) = frontline_registry.get_frontline_for_war(war.id).cloned() {
                let mut plan = match frontline_registry
                    .get_plan(fl.frontline_id, country_id)
                    .cloned()
                {
                    Some(p) => p,
                    None => continue,
                };

                let (own_p, enemy_p) =
                    evaluate_powers(&fl, &plan, country_id, military_registry, state_registry);
                total_own_power += own_p;
                total_enemy_power += enemy_p;

                // 攻勢目標の検証または選定
                let objective = select_ai_offensive_objective(
                    war,
                    country_id,
                    state_registry,
                    country_registry,
                    &fl,
                );
                plan.objective_region_id = objective;

                let Some(country) = country_registry.get(country_id) else {
                    continue;
                };

                // P28-003: 配送判定だけに使う母集合(戦力比・割当・即応判定は`plan`のまま)。
                let delivery_division_ids = offensive_delivery_division_ids(
                    &plan.assigned_division_ids,
                    country_id,
                    &capital_connected,
                    military_registry,
                    state_registry,
                );
                let (new_stance, reason, new_offensive_supply_blocked) =
                    evaluate_military_ai_decision(
                        &fl,
                        &plan,
                        &delivery_division_ids,
                        own_p,
                        enemy_p,
                        objective.is_some(),
                        military_registry,
                        country,
                        war_registry,
                    );

                last_reason = reason;
                plan.stance = new_stance;
                plan.offensive_supply_blocked = new_offensive_supply_blocked;

                // 前線レジストリへ更新を書き戻す
                if let Some(p_mut) = frontline_registry.get_plan_mut(fl.frontline_id, country_id) {
                    p_mut.stance = plan.stance;
                    p_mut.objective_region_id = plan.objective_region_id;
                    p_mut.offensive_supply_blocked = plan.offensive_supply_blocked;
                }
            }
        }

        let ai_state_mut = ai_registry.get_or_create_mut(country_id);
        ai_state_mut.last_evaluated_day = current_day;
        ai_state_mut.last_decision_reason = last_reason;
        ai_state_mut.estimated_own_power = total_own_power;
        ai_state_mut.estimated_enemy_power = total_enemy_power;
        ai_state_mut.dirty = skipped_due_to_start_date;
    }
}

/// System for daily military AI evaluation
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_military_ai(
    mut day_events: MessageReader<DayChangedMessage>,
    player_country: Res<PlayerCountry>,
    country_registry: Res<CountryRegistry>,
    war_registry: Res<WarRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    mut ai_registry: ResMut<MilitaryAiRegistry>,
) {
    for event in day_events.read() {
        let current_date = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);
        let current_day_num =
            (event.year * 365 + event.month as i32 * 30 + event.day as i32) as u32;

        process_daily_military_ai(
            current_day_num,
            &current_date,
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
    use crate::country::CountryData;
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{
        Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType,
    };
    use crate::state::data::StateData;
    use crate::war::frontline::update_all_frontlines;

    fn setup_test_env() -> (
        PlayerCountry,
        CountryRegistry,
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
        MilitaryAiRegistry,
    ) {
        let player_country = PlayerCountry(Some(CountryId(1)));

        let c1 = CountryData {
            id: CountryId(1),
            name: "Player C1".to_string(),
            capital_state_id: StateId(1),
            ..default()
        };
        let mut c2 = CountryData {
            id: CountryId(2),
            name: "AI C2".to_string(),
            capital_state_id: StateId(4),
            ..default()
        };
        // P26-008-B: `CountryData::default()`のMilitarySupplies備蓄は0であり、
        // このテスト環境が追加するDivision(需要>0)と組み合わせると、本タスクが
        // 追加した攻勢補給ゲートが即座にblockedと判定してしまう(この環境自体は
        // 補給を検証対象にしていない、旧来の「補給は無限」という暗黙の前提)。
        // 既存の戦力比ヒステリシス検証を妨げないよう、十分な備蓄を明示的に与える。
        c2.stockpile.set(
            crate::economy::resources::ResourceType::MilitarySupplies,
            1_000_000.0,
        );
        let c3 = CountryData {
            id: CountryId(3),
            name: "Neutral C3".to_string(),
            capital_state_id: StateId(5),
            ..default()
        };

        let country_registry = CountryRegistry {
            countries: vec![c1, c2, c3],
        };

        // 1(C1) -- 2(C1) -- 3(C2) -- 4(C2)
        let s1 = StateData {
            id: StateId(1),
            name: "State 1".to_string(),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s2 = StateData {
            id: StateId(2),
            name: "State 2".to_string(),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(1), StateId(3)],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "State 3".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(2), StateId(4)],
            world_position: [200.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s4 = StateData {
            id: StateId(4),
            name: "State 4".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(3)],
            world_position: [300.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s5 = StateData {
            id: StateId(5),
            name: "State 5".to_string(),
            owner_country_id: CountryId(3),
            neighbors: Vec::new(),
            world_position: [400.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3, s4, s5]);

        let mut military_registry = MilitaryRegistry::default();
        military_registry.definitions.insert(
            DivisionDefinitionId(1),
            DivisionDefinition {
                id: DivisionDefinitionId(1),
                name: "Infantry".to_string(),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                required_manpower: 1000,
                required_equipment: 10.0,
                recruitment_days: 10,
                movement_speed: 1.0,
                attack: 10.0,
                defense: 10.0,
                breakthrough: 5.0,
                organization: 100.0,
                morale: 1.0,
                supply_usage: 1.0,
                maintenance_cost: 1.0,
            },
        );

        let mut war_registry = WarRegistry::default();
        let war = War {
            id: WarId(1),
            name: "C1 vs C2".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            source_crisis_id: None,
            war_goals: vec![WarGoal {
                attacker: CountryId(1),
                defender: CountryId(2),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(3)],
                base_peace_cost: 20.0,
                international_concern: 10.0,
                completion: 0.0,
                is_primary: true,
            }],
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
        };
        war_registry.wars.insert(war.id, war);

        let frontline_registry = FrontlineRegistry::default();
        let ai_registry = MilitaryAiRegistry::default();

        (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
            ai_registry,
        )
    }

    #[test]
    fn test_ai_target_country_filtering() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        // プレイヤー(CountryId(1)) のAI状態は生成されない
        assert!(!ai_registry.ai_states.contains_key(&CountryId(1)));

        // AI国家(CountryId(2)) の状態が評価される
        assert!(ai_registry.ai_states.contains_key(&CountryId(2)));

        // 戦争非参加の中立国(CountryId(3)) の理由は NoActiveWar
        let c3_state = ai_registry.ai_states.get(&CountryId(3)).unwrap();
        assert_eq!(
            c3_state.last_decision_reason,
            MilitaryAiDecisionReason::NoActiveWar
        );
    }

    #[test]
    fn test_ai_division_assignment_and_power_evaluation() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        // C2 (AI国家) の部隊を追加
        let a_c2 = Division {
            id: DivisionId(0),
            owner: CountryId(2),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(4),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let c2_division_id = military_registry.add_division(a_c2);

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(1))
            .unwrap()
            .frontline_id;
        let plan_c2 = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();

        // C2 の陸軍が前線へ自動割当されたことを確認
        assert!(plan_c2.assigned_division_ids.contains(&c2_division_id));
    }

    #[test]
    fn test_ai_decision_thresholds_and_hysteresis() {
        let (
            player_country,
            country_registry,
            state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            mut ai_registry,
        ) = setup_test_env();

        // C2 (AI国家) に強力な陸軍を追加 (2000人)
        let a_c2 = Division {
            id: DivisionId(0),
            owner: CountryId(2),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(4),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 2000,
            max_manpower: 2000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let c2_division_id = military_registry.add_division(a_c2);

        // C1 (敵国) に弱い陸軍を追加 (500人)
        let a_c1 = Division {
            id: DivisionId(0),
            owner: CountryId(1),
            manpower: 500,
            max_manpower: 500,
            organization: 100.0,
            max_organization: 100.0,
            status: DivisionStatus::Idle,
            ..military_registry
                .divisions
                .get(&c2_division_id)
                .unwrap()
                .clone()
        };
        military_registry.add_division(a_c1);

        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        process_daily_military_ai(
            1,
            "1800/01/02",
            &player_country,
            &country_registry,
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
            &mut ai_registry,
        );

        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(1))
            .unwrap()
            .frontline_id;
        let plan_c2 = frontline_registry.get_plan(fl_id, CountryId(2)).unwrap();

        // 2000 vs 500 (> 110%) なので Offensive になる
        assert_eq!(plan_c2.stance, FrontlineStance::Offensive);
        let ai_c2 = ai_registry.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(
            ai_c2.last_decision_reason,
            MilitaryAiDecisionReason::SufficientAdvantage
        );
    }

    // ─── P28-003: 飛び地に取り残されたDivisionの前線配送判定からの除外 ───────────

    const P28_AI: CountryId = CountryId(2);
    const P28_ENEMY: CountryId = CountryId(1);

    fn p28_division(id: usize, owner: CountryId, state: StateId) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 1000,
            max_manpower: 1000,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn p28_state(id: usize, owner: CountryId, controller: Option<CountryId>, neighbors: Vec<usize>) -> StateData {
        StateData {
            id: StateId(id),
            name: format!("State {id}"),
            owner_country_id: owner,
            controller_country: controller,
            neighbors: neighbors.into_iter().map(StateId).collect(),
            world_position: [id as f32 * 100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        }
    }

    /// 1(C1)-2(C1)-3(C2)-4(C2首都)-9(C2)-10(C2)の本土。州6(C2所有・支配)と州7(C1所有、
    /// C2が占領)は本土と隣接しない飛び地。州8(C2所有、C1が占領)は州4に隣接する。
    fn p28_states(nine_controller: Option<CountryId>) -> StateRegistry {
        StateRegistry::build(vec![
            p28_state(1, P28_ENEMY, None, vec![2]),
            p28_state(2, P28_ENEMY, None, vec![1, 3]),
            p28_state(3, P28_AI, None, vec![2, 4]),
            p28_state(4, P28_AI, None, vec![3, 9, 8]),
            p28_state(6, P28_AI, None, vec![]),
            p28_state(7, P28_ENEMY, Some(P28_AI), vec![]),
            p28_state(8, P28_AI, Some(P28_ENEMY), vec![4]),
            p28_state(9, P28_AI, nine_controller, vec![4, 10]),
            p28_state(10, P28_AI, None, vec![9]),
        ])
    }

    fn p28_military(divisions: &[(usize, StateId)]) -> MilitaryRegistry {
        let (_, _, _, _, mut military, _, _) = setup_test_env();
        for &(id, state) in divisions {
            military.divisions.insert(DivisionId(id), p28_division(id, P28_AI, state));
        }
        military
    }

    fn p28_scope(assigned: &[usize], states: &StateRegistry, military: &MilitaryRegistry) -> Vec<DivisionId> {
        let connected =
            crate::military::capital_connectivity::connected_states_from_capital(StateId(4), P28_AI, states);
        let ids: Vec<DivisionId> = assigned.iter().copied().map(DivisionId).collect();
        offensive_delivery_division_ids(&ids, P28_AI, &connected, military, states)
    }

    #[test]
    fn p28_connected_friendly_divisions_stay_in_the_delivery_scope() {
        let states = p28_states(None);
        let military = p28_military(&[(1, StateId(3)), (2, StateId(4)), (3, StateId(10))]);
        assert_eq!(p28_scope(&[1, 2, 3], &states, &military), vec![DivisionId(1), DivisionId(2), DivisionId(3)]);
    }

    #[test]
    fn p28_stranded_friendly_exclave_divisions_are_removed_when_connected_ones_remain() {
        let states = p28_states(None);
        // 州6: 自国所有・自国支配の飛び地。州7: 他国所有だが自国が支配する飛び地。
        let military = p28_military(&[(1, StateId(3)), (2, StateId(6)), (3, StateId(7))]);
        assert_eq!(p28_scope(&[1, 2, 3], &states, &military), vec![DivisionId(1)]);
    }

    #[test]
    fn p28_all_stranded_front_keeps_the_original_set_instead_of_an_empty_scope() {
        let states = p28_states(None);
        let military = p28_military(&[(2, StateId(6)), (3, StateId(7))]);
        assert_eq!(
            p28_scope(&[2, 3], &states, &military),
            vec![DivisionId(2), DivisionId(3)],
            "補給不能な部隊しかいない前線を空集合として扱わない"
        );
        assert!(p28_scope(&[], &states, &military).is_empty(), "割当0件は従来どおり空のまま");
    }

    #[test]
    fn p28_divisions_in_enemy_controlled_states_are_never_removed() {
        let states = p28_states(None);
        // 州2: 敵国支配(攻撃中)。州8: 自国所有だが敵が占領中(controller≠自国)。
        let military = p28_military(&[(1, StateId(2)), (2, StateId(8)), (3, StateId(6))]);
        assert_eq!(p28_scope(&[1, 2, 3], &states, &military), vec![DivisionId(1), DivisionId(2)]);
    }

    #[test]
    fn p28_scope_follows_the_current_controller_when_the_route_is_cut_and_restored() {
        let military = p28_military(&[(1, StateId(3)), (2, StateId(10))]);
        // 経路上の州9を敵が占領すると州10は首都から切断され、配送判定から外れる。
        let cut = p28_states(Some(P28_ENEMY));
        assert_eq!(p28_scope(&[1, 2], &cut, &military), vec![DivisionId(1)]);
        // 州9を取り戻すと再び連結し、配送判定に戻る。
        let restored = p28_states(None);
        assert_eq!(p28_scope(&[1, 2], &restored, &military), vec![DivisionId(1), DivisionId(2)]);
    }

    #[test]
    fn p28_scope_does_not_depend_on_registration_or_neighbor_order() {
        let military = p28_military(&[(1, StateId(3)), (2, StateId(6)), (3, StateId(10))]);
        let forward = p28_scope(&[1, 2, 3], &p28_states(None), &military);
        let mut reversed_states: Vec<StateData> = p28_states(None).states.into_iter().rev().collect();
        for s in &mut reversed_states {
            s.neighbors.reverse();
        }
        let reversed = p28_scope(&[3, 2, 1], &StateRegistry::build(reversed_states), &military);
        let mut a = forward.clone();
        let mut b = reversed.clone();
        a.sort_by_key(|d| d.0);
        b.sort_by_key(|d| d.0);
        assert_eq!(a, b);
        assert_eq!(a, vec![DivisionId(1), DivisionId(3)]);
    }

    /// 配送判定の母集合だけが変わり、戦力比・割当・即応判定は変わらないことを
    /// `evaluate_military_ai_decision`経由で確認する。
    #[test]
    fn p28_stranded_zero_fulfillment_no_longer_blocks_an_otherwise_ready_offensive() {
        let (_, country_registry, _, war_registry, _, mut frontline_registry, _) = setup_test_env();
        let states = p28_states(None);
        let mut military = p28_military(&[(1, StateId(3)), (2, StateId(6))]);
        military.division_supply_fulfillment.insert(DivisionId(1), 1.0);
        military.division_supply_fulfillment.insert(DivisionId(2), 0.0);
        update_all_frontlines(&war_registry, &states, &military, &mut frontline_registry);
        let fl = frontline_registry.get_frontline_for_war(WarId(1)).unwrap().clone();
        let mut plan = frontline_registry.get_plan(fl.frontline_id, P28_AI).unwrap().clone();
        plan.assigned_division_ids = vec![DivisionId(1), DivisionId(2)];
        let country = country_registry.get(P28_AI).unwrap();
        let connected =
            crate::military::capital_connectivity::connected_states_from_capital(StateId(4), P28_AI, &states);

        // 従来の母集合(割当全体)では、飛び地の充足率0が最小値になり補給不足で止まる。
        let (stance, reason, blocked) = evaluate_military_ai_decision(
            &fl,
            &plan,
            &plan.assigned_division_ids,
            2000,
            1000,
            true,
            &military,
            country,
            &war_registry,
        );
        assert_eq!((stance, reason, blocked), (FrontlineStance::Defend, MilitaryAiDecisionReason::SupplyShortage, true));

        // P28-003の母集合では、連結したDivisionの充足率1.0で判定し、攻勢を開始できる。
        let scope = offensive_delivery_division_ids(&plan.assigned_division_ids, P28_AI, &connected, &military, &states);
        assert_eq!(scope, vec![DivisionId(1)]);
        let (stance, reason, blocked) =
            evaluate_military_ai_decision(&fl, &plan, &scope, 2000, 1000, true, &military, country, &war_registry);
        assert_eq!((stance, reason, blocked), (FrontlineStance::Offensive, MilitaryAiDecisionReason::SufficientAdvantage, false));

        // 110%の戦力比条件は割当全体の戦力(呼び出し側の値)のまま: 1:1では攻勢を開始しない。
        let (stance, reason, _) =
            evaluate_military_ai_decision(&fl, &plan, &scope, 1000, 1000, true, &military, country, &war_registry);
        assert_eq!((stance, reason), (FrontlineStance::Defend, MilitaryAiDecisionReason::ComparableStrength));

        // 全員が取り残された前線は元の集合(充足率0)で判定し、空集合として攻勢許可しない。
        plan.assigned_division_ids = vec![DivisionId(2)];
        let scope = offensive_delivery_division_ids(&plan.assigned_division_ids, P28_AI, &connected, &military, &states);
        let (stance, reason, _) =
            evaluate_military_ai_decision(&fl, &plan, &scope, 2000, 1000, true, &military, country, &war_registry);
        assert_eq!((stance, reason), (FrontlineStance::Defend, MilitaryAiDecisionReason::SupplyShortage));
    }
}
