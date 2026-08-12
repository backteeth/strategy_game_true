use crate::app::time::DayChangedMessage;
use crate::common::{ArmyId, CountryId, FrontlineId, StateId, WarId};
use crate::military::data::{ArmyStatus, MilitaryRegistry};
use crate::military::pathfinding::find_path;
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 前線の命令状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FrontlineStance {
    #[default]
    Stopped,
    Defend,
    Offensive,
}

impl FrontlineStance {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            FrontlineStance::Stopped => "frontline_stance.stopped",
            FrontlineStance::Defend => "frontline_stance.defend",
            FrontlineStance::Offensive => "frontline_stance.offensive",
        }
    }
}

/// 前線データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontline {
    pub frontline_id: FrontlineId,
    pub war_id: WarId,
    pub attacker_country_id: CountryId,
    pub defender_country_id: CountryId,
    /// 攻撃側国境地域（StateId昇順で安定保持）
    pub attacker_front_regions: Vec<StateId>,
    /// 防御側国境地域（StateId昇順で安定保持）
    pub defender_front_regions: Vec<StateId>,
    /// 国境ペアリスト (attacker_state, defender_state) （安定順）
    pub border_region_pairs: Vec<(StateId, StateId)>,
}

/// 作戦命令データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontlinePlan {
    pub frontline_id: FrontlineId,
    pub commanding_country_id: CountryId,
    pub stance: FrontlineStance,
    pub objective_region_id: Option<StateId>,
    pub assigned_army_ids: Vec<ArmyId>,
}

impl FrontlinePlan {
    pub fn new(frontline_id: FrontlineId, commanding_country_id: CountryId) -> Self {
        Self {
            frontline_id,
            commanding_country_id,
            stance: FrontlineStance::Stopped,
            objective_region_id: None,
            assigned_army_ids: Vec::new(),
        }
    }
}

/// 前線と作戦命令を集中管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct FrontlineRegistry {
    pub frontlines: HashMap<FrontlineId, Frontline>,
    /// key: (FrontlineId, CountryId)
    pub plans: HashMap<(FrontlineId, CountryId), FrontlinePlan>,
    pub next_frontline_id: usize,
    /// ArmyId -> FrontlineId のマッピング（1陸軍は1前線のみ所属）
    pub army_frontline_map: HashMap<ArmyId, FrontlineId>,
    /// 前線によって自動生成された移動命令を実行中の陸軍セット
    pub frontline_generated_movements: HashSet<ArmyId>,
}

impl FrontlineRegistry {
    /// 新しい前線IDを発行する
    pub fn generate_id(&mut self) -> FrontlineId {
        let id = FrontlineId(self.next_frontline_id);
        self.next_frontline_id += 1;
        id
    }

    /// 特定戦争に対応する前線を取得
    pub fn get_frontline_for_war(&self, war_id: WarId) -> Option<&Frontline> {
        self.frontlines.values().find(|f| f.war_id == war_id)
    }

    /// 特定戦争に対応する前線を取得（可変）
    pub fn get_frontline_for_war_mut(&mut self, war_id: WarId) -> Option<&mut Frontline> {
        self.frontlines.values_mut().find(|f| f.war_id == war_id)
    }

    /// 国家の作戦命令を取得
    pub fn get_plan(
        &self,
        frontline_id: FrontlineId,
        country_id: CountryId,
    ) -> Option<&FrontlinePlan> {
        self.plans.get(&(frontline_id, country_id))
    }

    /// 国家の作戦命令を取得（可変）
    pub fn get_plan_mut(
        &mut self,
        frontline_id: FrontlineId,
        country_id: CountryId,
    ) -> Option<&mut FrontlinePlan> {
        self.plans.get_mut(&(frontline_id, country_id))
    }

    /// 陸軍を前線へ割り当てる
    pub fn assign_army(
        &mut self,
        army_id: ArmyId,
        frontline_id: FrontlineId,
        country_id: CountryId,
        military_registry: &MilitaryRegistry,
        war_registry: &WarRegistry,
    ) -> Result<(), &'static str> {
        let frontline = self
            .frontlines
            .get(&frontline_id)
            .ok_or("Frontline not found")?;
        let war = war_registry
            .wars
            .get(&frontline.war_id)
            .ok_or("War not found")?;

        if war.status != WarStatus::Active {
            return Err("War is not active");
        }

        if country_id != frontline.attacker_country_id
            && country_id != frontline.defender_country_id
        {
            return Err("Country is not a participant in this frontline");
        }

        let army = military_registry
            .armies
            .get(&army_id)
            .ok_or("Army not found")?;
        if army.owner != country_id {
            return Err("Army belongs to a different country");
        }
        if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
            return Err("Army is destroyed or has 0 manpower");
        }

        // 他の前線からの割当解除
        if let Some(old_fl_id) = self.army_frontline_map.remove(&army_id)
            && let Some(old_plan) = self.plans.get_mut(&(old_fl_id, country_id))
        {
            old_plan.assigned_army_ids.retain(|&id| id != army_id);
        }

        // 対象Planの取得または作成
        let plan = self
            .plans
            .entry((frontline_id, country_id))
            .or_insert_with(|| FrontlinePlan::new(frontline_id, country_id));

        if !plan.assigned_army_ids.contains(&army_id) {
            plan.assigned_army_ids.push(army_id);
            plan.assigned_army_ids.sort_by_key(|id| id.0);
        }

        self.army_frontline_map.insert(army_id, frontline_id);
        Ok(())
    }

    /// 陸軍の前線割り当てを解除する
    ///
    /// `country_id`の陸軍であることを検証してから解除する。P21-002以前は所有者検証が
    /// 存在せず、選択中の陸軍(所有者不問で選択可能、`map::army_selection`参照)が
    /// 敵国や第三国の陸軍であっても無条件に前線割り当てを解除できてしまっていた。
    pub fn unassign_army(
        &mut self,
        army_id: ArmyId,
        country_id: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let army = military_registry
            .armies
            .get(&army_id)
            .ok_or("Army not found")?;
        if army.owner != country_id {
            return Err("Army belongs to a different country");
        }

        if let Some(fl_id) = self.army_frontline_map.remove(&army_id) {
            for plan in self.plans.values_mut() {
                if plan.frontline_id == fl_id {
                    plan.assigned_army_ids.retain(|&id| id != army_id);
                }
            }
        }
        self.frontline_generated_movements.remove(&army_id);
        Ok(())
    }

    /// 前線の全陸軍割当を解除する
    pub fn unassign_all_armies_for_plan(
        &mut self,
        frontline_id: FrontlineId,
        country_id: CountryId,
    ) {
        if let Some(plan) = self.plans.get_mut(&(frontline_id, country_id)) {
            for army_id in plan.assigned_army_ids.drain(..) {
                self.army_frontline_map.remove(&army_id);
                self.frontline_generated_movements.remove(&army_id);
            }
        }
    }

    /// 無効になった陸軍・前線の参照を整理
    pub fn sanitize_references(
        &mut self,
        military_registry: &MilitaryRegistry,
        war_registry: &WarRegistry,
    ) {
        // 無効な戦争の前線を収集
        let invalid_fl_ids: Vec<FrontlineId> = self
            .frontlines
            .values()
            .filter(|fl| {
                war_registry
                    .wars
                    .get(&fl.war_id)
                    .map(|w| w.status != WarStatus::Active)
                    .unwrap_or(true)
            })
            .map(|fl| fl.frontline_id)
            .collect();

        for fl_id in invalid_fl_ids {
            self.remove_frontline(fl_id, military_registry);
        }

        // 各Plan内の無効陸軍（削除済み、戦力0）の整理
        for plan in self.plans.values_mut() {
            plan.assigned_army_ids.retain(|&army_id| {
                if let Some(army) = military_registry.armies.get(&army_id) {
                    army.manpower > 0
                        && army.status != ArmyStatus::Destroyed
                        && army.owner == plan.commanding_country_id
                } else {
                    false
                }
            });
        }

        // army_frontline_map の整理
        self.army_frontline_map.retain(|army_id, fl_id| {
            if !self.frontlines.contains_key(fl_id) {
                return false;
            }
            if let Some(army) = military_registry.armies.get(army_id) {
                army.manpower > 0 && army.status != ArmyStatus::Destroyed
            } else {
                false
            }
        });

        // frontline_generated_movements の整理
        self.frontline_generated_movements
            .retain(|army_id| military_registry.armies.contains_key(army_id));
    }

    /// 前線と関連データを削除する
    pub fn remove_frontline(
        &mut self,
        frontline_id: FrontlineId,
        military_registry: &MilitaryRegistry,
    ) {
        self.frontlines.remove(&frontline_id);

        let plan_keys: Vec<(FrontlineId, CountryId)> = self
            .plans
            .keys()
            .filter(|(fl_id, _)| *fl_id == frontline_id)
            .copied()
            .collect();

        for key in plan_keys {
            if let Some(plan) = self.plans.remove(&key) {
                for army_id in plan.assigned_army_ids {
                    self.army_frontline_map.remove(&army_id);
                    self.frontline_generated_movements.remove(&army_id);
                }
            }
        }

        // frontline_generated_movementsの解除と、該当陸軍の前線移動停止
        let _ = military_registry;
    }
}

/// P21-002: 前線割当/解除ボタンが実行可能かを副作用なしで判定する結果。
/// UIの表示更新(ボタン活性/非活性)とクリックハンドラの双方から利用する
/// (`military::recruitment::RecruitFeasibility`と同型のパターン)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontlineCommandFeasibility {
    Ready,
    NoActiveFrontline,
    NoArmySelected,
    ArmyNotFound,
    NotOwnArmy,
    ArmyDestroyed,
}

impl FrontlineCommandFeasibility {
    pub fn is_ready(self) -> bool {
        matches!(self, FrontlineCommandFeasibility::Ready)
    }
}

/// 選択中の陸軍を前線へ割当/解除する操作が可能かを判定する。
///
/// `selected_army_id`は所有者を問わず選択され得る(`map::army_selection::handle_army_selection`
/// 参照)ため、ここで所有者を必ず再検証する。`assign_army`/`unassign_army`実行時にも
/// 同じ検証を行うため、この関数の結果は表示専用であり実行の可否を最終的に保証するのは
/// 実行系側(`assign_army`/`unassign_army`自身)である。
fn evaluate_single_army_command_feasibility(
    army_id: ArmyId,
    player_cid: CountryId,
    military_registry: &MilitaryRegistry,
) -> FrontlineCommandFeasibility {
    let Some(army) = military_registry.armies.get(&army_id) else {
        return FrontlineCommandFeasibility::ArmyNotFound;
    };
    if army.owner != player_cid {
        return FrontlineCommandFeasibility::NotOwnArmy;
    }
    if army.manpower == 0 || army.status == ArmyStatus::Destroyed {
        return FrontlineCommandFeasibility::ArmyDestroyed;
    }
    FrontlineCommandFeasibility::Ready
}

/// P21-003: 複数選択に対応。`selected_army_ids`のうち1件でも実行可能ならReadyを返す
/// (複数選択時は一部の陸軍だけでも割当/解除が成立すれば十分なため)。全滅なら、
/// `selected_army_ids`をArmyId昇順に評価した最初の失敗理由を返す(呼び出し側が
/// ソート済みの配列を渡すことで結果を決定的にする)。
pub fn evaluate_frontline_army_command_feasibility(
    selected_army_ids: &[ArmyId],
    player_cid: CountryId,
    military_registry: &MilitaryRegistry,
    frontline: Option<&Frontline>,
) -> FrontlineCommandFeasibility {
    if frontline.is_none() {
        return FrontlineCommandFeasibility::NoActiveFrontline;
    }
    if selected_army_ids.is_empty() {
        return FrontlineCommandFeasibility::NoArmySelected;
    }

    let mut first_failure = FrontlineCommandFeasibility::NoArmySelected;
    for &army_id in selected_army_ids {
        let result =
            evaluate_single_army_command_feasibility(army_id, player_cid, military_registry);
        if result == FrontlineCommandFeasibility::Ready {
            return FrontlineCommandFeasibility::Ready;
        }
        if first_failure == FrontlineCommandFeasibility::NoArmySelected {
            first_failure = result;
        }
    }
    first_failure
}

/// 前線境界を実効支配地域から決定的に計算する
pub fn calculate_frontline_border(
    war: &War,
    state_registry: &StateRegistry,
) -> (Vec<StateId>, Vec<StateId>, Vec<(StateId, StateId)>) {
    let attacker = match war.attackers.iter().next() {
        Some(&c) => c,
        None => return (Vec::new(), Vec::new(), Vec::new()),
    };
    let defender = match war.defenders.iter().next() {
        Some(&c) => c,
        None => return (Vec::new(), Vec::new(), Vec::new()),
    };

    let mut border_pairs: Vec<(StateId, StateId)> = Vec::new();
    let mut attacker_front_set = HashSet::new();
    let mut defender_front_set = HashSet::new();

    // 安定した地域探索のためStateDataの順序を一定（StateId昇順）にする
    let mut states: Vec<&crate::state::data::StateData> = state_registry.states.iter().collect();
    states.sort_by_key(|s| s.id.0);

    for state in states {
        if state.is_sea {
            continue;
        }

        let controller = state.controller();
        if controller != attacker && controller != defender {
            continue;
        }

        // 隣接地域を確認
        let mut neighbors = state.neighbors.clone();
        neighbors.sort_by_key(|s| s.0);

        for neighbor_id in neighbors {
            let neighbor_state = match state_registry.get(neighbor_id) {
                Some(s) => s,
                None => continue,
            };

            if neighbor_state.is_sea {
                continue;
            }

            let neighbor_controller = neighbor_state.controller();

            if controller == attacker && neighbor_controller == defender {
                border_pairs.push((state.id, neighbor_state.id));
                attacker_front_set.insert(state.id);
                defender_front_set.insert(neighbor_state.id);
            }
        }
    }

    border_pairs.sort_by(|a, b| a.0.0.cmp(&b.0.0).then_with(|| a.1.0.cmp(&b.1.0)));
    border_pairs.dedup();

    let mut attacker_front: Vec<StateId> = attacker_front_set.into_iter().collect();
    attacker_front.sort_by_key(|s| s.0);

    let mut defender_front: Vec<StateId> = defender_front_set.into_iter().collect();
    defender_front.sort_by_key(|s| s.0);

    (attacker_front, defender_front, border_pairs)
}

/// 進行中戦争の前線を生成または再計算する
pub fn update_or_create_frontline_for_war(
    war: &War,
    state_registry: &StateRegistry,
    frontline_registry: &mut FrontlineRegistry,
) -> FrontlineId {
    let attacker = *war.attackers.iter().next().unwrap();
    let defender = *war.defenders.iter().next().unwrap();

    let (atk_front, def_front, pairs) = calculate_frontline_border(war, state_registry);

    if let Some(existing_fl) = frontline_registry.get_frontline_for_war_mut(war.id) {
        existing_fl.attacker_front_regions = atk_front;
        existing_fl.defender_front_regions = def_front;
        existing_fl.border_region_pairs = pairs;
        existing_fl.frontline_id
    } else {
        let fl_id = frontline_registry.generate_id();
        let frontline = Frontline {
            frontline_id: fl_id,
            war_id: war.id,
            attacker_country_id: attacker,
            defender_country_id: defender,
            attacker_front_regions: atk_front,
            defender_front_regions: def_front,
            border_region_pairs: pairs,
        };
        frontline_registry.frontlines.insert(fl_id, frontline);

        // 初期Planの作成
        frontline_registry
            .plans
            .entry((fl_id, attacker))
            .or_insert_with(|| FrontlinePlan::new(fl_id, attacker));
        frontline_registry
            .plans
            .entry((fl_id, defender))
            .or_insert_with(|| FrontlinePlan::new(fl_id, defender));

        fl_id
    }
}

/// 全アクティブ戦争の前線を計算・更新
pub fn update_all_frontlines(
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
) {
    frontline_registry.sanitize_references(military_registry, war_registry);

    let active_wars: Vec<&War> = war_registry
        .wars
        .values()
        .filter(|w| w.status == WarStatus::Active)
        .collect();

    for war in active_wars {
        update_or_create_frontline_for_war(war, state_registry, frontline_registry);
    }
}

/// 攻勢目標地域を選択または決定する
pub fn determine_offensive_objective(
    war: &War,
    commanding_country: CountryId,
    state_registry: &StateRegistry,
) -> Option<StateId> {
    let attacker = *war.attackers.iter().next()?;
    let defender = *war.defenders.iter().next()?;

    if commanding_country == attacker {
        // 攻撃側初期目標: 戦争目標地域
        if let Some(primary_goal) = war.war_goals.first()
            && let Some(&target_state) = primary_goal.target_states.first()
            && let Some(state) = state_registry.get(target_state)
            && !state.is_sea
        {
            return Some(target_state);
        }
    } else if commanding_country == defender {
        // 防御側優先1: 防御側が法的に所有しているが攻撃側に占領されている地域
        let mut occupied_defender_states: Vec<StateId> = state_registry
            .states
            .iter()
            .filter(|s| !s.is_sea && s.owner_country_id == defender && s.controller() == attacker)
            .map(|s| s.id)
            .collect();
        occupied_defender_states.sort_by_key(|s| s.0);

        if !occupied_defender_states.is_empty() {
            // その中に戦争目標地域があればそれを最優先
            if let Some(primary_goal) = war.war_goals.first()
                && let Some(&target_state) = primary_goal.target_states.first()
                && occupied_defender_states.contains(&target_state)
            {
                return Some(target_state);
            }
            return Some(occupied_defender_states[0]);
        }

        // 防御側優先2: 奪回対象がなければ、攻撃側が法的に所有する陸上地域で最小StateId
        let mut attacker_owned_states: Vec<StateId> = state_registry
            .states
            .iter()
            .filter(|s| !s.is_sea && s.owner_country_id == attacker)
            .map(|s| s.id)
            .collect();
        attacker_owned_states.sort_by_key(|s| s.0);

        if !attacker_owned_states.is_empty() {
            return Some(attacker_owned_states[0]);
        }
    }

    None
}

/// 防御配置処理 (Defend Stance)
pub fn process_defensive_plan(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    _war_registry: &WarRegistry,
    frontline_registry: &mut FrontlineRegistry,
) {
    let country_id = plan.commanding_country_id;
    let enemy_id = if country_id == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    let front_regions = if country_id == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    if front_regions.is_empty() || plan.assigned_army_ids.is_empty() {
        return;
    }

    // 1. 各前線地域に配置中・移動目的地中の割当陸軍数を集計
    let mut region_counts: HashMap<StateId, usize> =
        front_regions.iter().map(|&r| (r, 0)).collect();

    for &army_id in &plan.assigned_army_ids {
        if let Some(army) = military_registry.armies.get(&army_id) {
            let target = army.destination.unwrap_or(army.current_state);
            if region_counts.contains_key(&target) {
                *region_counts.get_mut(&target).unwrap() += 1;
            }
        }
    }

    // 2. 陸軍をArmyId順で処理し、最適な配置先へ移動指示
    let mut army_ids = plan.assigned_army_ids.clone();
    army_ids.sort_by_key(|id| id.0);

    for army_id in army_ids {
        let army = match military_registry.armies.get(&army_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        // 命令受給可能か判定
        if army.manpower == 0
            || army.status == ArmyStatus::Fighting
            || army.status == ArmyStatus::Retreating
            || army.status == ArmyStatus::Destroyed
        {
            continue;
        }

        // 手動移動中は上書きしない（frontline_generated_movementsに含まれず、かつMovingの場合はスキップ）
        if army.status == ArmyStatus::Moving
            && !frontline_registry
                .frontline_generated_movements
                .contains(&army_id)
        {
            continue;
        }

        // 既に前線地域におり、待機中の場合
        if front_regions.contains(&army.current_state) && army.status == ArmyStatus::Idle {
            // 現在地がまだ配置バランス上問題なければ維持
            continue;
        }

        // 移動中の場合はカウントを一旦仮減算して再配置決定
        let curr_target = army.destination.unwrap_or(army.current_state);
        if let Some(count) = region_counts.get_mut(&curr_target) {
            *count = count.saturating_sub(1);
        }

        // 最適配置先の選定
        // 条件: 1. 配置数最小 -> 2. 距離/ステップ数 -> 3. StateId最小
        let mut candidates: Vec<(StateId, usize, usize)> = Vec::new();

        for &region_id in front_regions {
            let count = *region_counts.get(&region_id).unwrap_or(&0);
            if let Some(path) = find_path(
                army.current_state,
                region_id,
                state_registry,
                &[country_id],
                &[enemy_id],
            ) {
                let dist = path.len();
                candidates.push((region_id, count, dist));
            }
        }

        candidates.sort_by(|a, b| {
            a.1.cmp(&b.1) // 配置数最小
                .then_with(|| a.2.cmp(&b.2)) // 経路長最小
                .then_with(|| a.0.0.cmp(&b.0.0)) // StateId最小
        });

        if let Some((best_region, _, _)) = candidates.first().copied() {
            *region_counts.entry(best_region).or_default() += 1;

            if army.current_state != best_region
                && let Some(path) = find_path(
                    army.current_state,
                    best_region,
                    state_registry,
                    &[country_id],
                    &[enemy_id],
                )
                && !path.is_empty()
                && let Some(a_mut) = military_registry.armies.get_mut(&army_id)
            {
                a_mut.destination = Some(best_region);
                a_mut.current_path = path;
                a_mut.target_state = None;
                a_mut.status = ArmyStatus::Moving;
                a_mut.movement_progress = 0.0;
                frontline_registry
                    .frontline_generated_movements
                    .insert(army_id);
            }
        }
    }
}

/// 攻勢命令処理 (Offensive Stance)
pub fn process_offensive_plan(
    frontline: &Frontline,
    plan: &mut FrontlinePlan,
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    war_registry: &WarRegistry,
    frontline_registry: &mut FrontlineRegistry,
) {
    let country_id = plan.commanding_country_id;
    let enemy_id = if country_id == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    let war = match war_registry.wars.get(&frontline.war_id) {
        Some(w) => w,
        None => return,
    };

    // 攻勢目標の確認
    let objective = match plan.objective_region_id {
        Some(obj) => obj,
        None => {
            let default_obj = determine_offensive_objective(war, country_id, state_registry);
            plan.objective_region_id = default_obj;
            match default_obj {
                Some(obj) => obj,
                None => return,
            }
        }
    };

    // 攻勢目標を自国が支配済みかチェック
    if let Some(obj_state) = state_registry.get(objective)
        && obj_state.controller() == country_id
    {
        // 目標達成！ Defend に自動移行
        plan.stance = FrontlineStance::Defend;
        process_defensive_plan(
            frontline,
            plan,
            state_registry,
            military_registry,
            war_registry,
            frontline_registry,
        );
        return;
    }

    // まず自国側前線へ陸軍を分散配置（移動中・待機中へ）
    process_defensive_plan(
        frontline,
        plan,
        state_registry,
        military_registry,
        war_registry,
        frontline_registry,
    );

    // 待機中 (Idle) で自国側前線地域にいる陸軍から攻撃命令を発行
    let front_regions = if country_id == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    let mut army_ids = plan.assigned_army_ids.clone();
    army_ids.sort_by_key(|id| id.0);

    let mut daily_target_counts: HashMap<StateId, usize> = HashMap::new();

    for army_id in army_ids {
        let army = match military_registry.armies.get(&army_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        if army.status != ArmyStatus::Idle || army.manpower == 0 {
            continue;
        }

        if !front_regions.contains(&army.current_state) {
            continue;
        }

        let curr_state = match state_registry.get(army.current_state) {
            Some(s) => s,
            None => continue,
        };

        // 隣接する敵支配陸上地域を攻撃候補として抽出
        let mut candidates: Vec<StateId> = curr_state
            .neighbors
            .iter()
            .copied()
            .filter(|&nid| {
                if let Some(ns) = state_registry.get(nid) {
                    !ns.is_sea && ns.controller() == enemy_id
                } else {
                    false
                }
            })
            .collect();

        if candidates.is_empty() {
            continue;
        }

        // 候補の優先順位決定
        // 1. 攻勢目標そのもの
        // 2. 攻勢目標までの最短グラフ距離
        // 3. 本日割り当て攻撃数が少ない地域
        // 4. StateId昇順
        candidates.sort_by(|&a, &b| {
            let a_is_obj = a == objective;
            let b_is_obj = b == objective;
            if a_is_obj != b_is_obj {
                return b_is_obj.cmp(&a_is_obj);
            }

            let a_dist = find_path(a, objective, state_registry, &[country_id], &[enemy_id])
                .map(|p| p.len())
                .unwrap_or(usize::MAX);
            let b_dist = find_path(b, objective, state_registry, &[country_id], &[enemy_id])
                .map(|p| p.len())
                .unwrap_or(usize::MAX);

            let a_cnt = *daily_target_counts.get(&a).unwrap_or(&0);
            let b_cnt = *daily_target_counts.get(&b).unwrap_or(&0);

            a_dist
                .cmp(&b_dist)
                .then_with(|| a_cnt.cmp(&b_cnt))
                .then_with(|| a.0.cmp(&b.0))
        });

        if let Some(&best_target) = candidates.first() {
            *daily_target_counts.entry(best_target).or_default() += 1;

            if let Some(a_mut) = military_registry.armies.get_mut(&army_id) {
                a_mut.destination = Some(best_target);
                a_mut.current_path = vec![best_target];
                a_mut.target_state = None;
                a_mut.status = ArmyStatus::Moving;
                a_mut.movement_progress = 0.0;
                frontline_registry
                    .frontline_generated_movements
                    .insert(army_id);
            }
        }
    }
}

/// 停止命令処理 (Stopped Stance)
pub fn process_stopped_plan(
    plan: &FrontlinePlan,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
) {
    for &army_id in &plan.assigned_army_ids {
        if frontline_registry
            .frontline_generated_movements
            .contains(&army_id)
            && let Some(army) = military_registry.armies.get_mut(&army_id)
            && army.status == ArmyStatus::Moving
        {
            army.status = ArmyStatus::Idle;
            army.destination = None;
            army.current_path.clear();
            army.target_state = None;
            army.movement_progress = 0.0;
        }
        frontline_registry
            .frontline_generated_movements
            .remove(&army_id);
    }
}

/// 日次作戦命令の実行
pub fn process_daily_frontline_plans(
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    current_date: Option<&str>,
) {
    update_all_frontlines(
        war_registry,
        state_registry,
        military_registry,
        frontline_registry,
    );

    let frontline_ids: Vec<FrontlineId> = frontline_registry.frontlines.keys().copied().collect();

    for fl_id in frontline_ids {
        let frontline = match frontline_registry.frontlines.get(&fl_id).cloned() {
            Some(fl) => fl,
            None => continue,
        };

        // 宣戦布告当日は前線命令による自動移動命令の生成をスキップ
        if let Some(curr) = current_date
            && let Some(war) = war_registry.wars.get(&frontline.war_id)
            && war.start_date == curr
        {
            continue;
        }

        // 攻撃側・防御側のPlanを処理
        let countries = [frontline.attacker_country_id, frontline.defender_country_id];

        for country_id in countries {
            let mut plan = match frontline_registry.get_plan(fl_id, country_id).cloned() {
                Some(p) => p,
                None => continue,
            };

            match plan.stance {
                FrontlineStance::Stopped => {
                    process_stopped_plan(&plan, military_registry, frontline_registry);
                }
                FrontlineStance::Defend => {
                    process_defensive_plan(
                        &frontline,
                        &plan,
                        state_registry,
                        military_registry,
                        war_registry,
                        frontline_registry,
                    );
                }
                FrontlineStance::Offensive => {
                    process_offensive_plan(
                        &frontline,
                        &mut plan,
                        state_registry,
                        military_registry,
                        war_registry,
                        frontline_registry,
                    );
                    // 変更されたPlanを反映（自動Defend切り替えや攻勢目標設定のため）
                    if let Some(p_mut) = frontline_registry.get_plan_mut(fl_id, country_id) {
                        p_mut.stance = plan.stance;
                        p_mut.objective_region_id = plan.objective_region_id;
                    }
                }
            }
        }
    }
}

/// System for daily frontline plan execution
pub fn handle_daily_frontline_plans(
    mut day_events: MessageReader<DayChangedMessage>,
    war_registry: Res<WarRegistry>,
    state_registry: Res<StateRegistry>,
    mut military_registry: ResMut<MilitaryRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
    date: Res<crate::app::time::GameDate>,
) {
    for _ in day_events.read() {
        let current_date = date.display();
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            Some(&current_date),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ArmyId, CountryId, DivisionId, StateId, WarId};
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{
        ArmyStatus, ArmyUnit, DivisionDefinition, DivisionSize, DivisionType,
    };
    use crate::state::data::StateData;

    fn setup_test_environment() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
    ) {
        let mut war_registry = WarRegistry::default();
        let mut military_registry = MilitaryRegistry::default();
        let frontline_registry = FrontlineRegistry::default();

        // 4つの陸上地域 (State 1..=4) と 1つの海域 (State 5)
        // 1(C1) -- 2(C1) -- 3(C2) -- 4(C2)
        // State 5(Sea) 2と3に隣接
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
            neighbors: vec![StateId(1), StateId(3), StateId(5)],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "State 3".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(2), StateId(4), StateId(5)],
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
            name: "Sea 5".to_string(),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(2), StateId(3)],
            is_sea: true,
            world_position: [150.0, 100.0],
            size: [100.0, 100.0],
            ..default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3, s4, s5]);

        // 師団定義
        military_registry.definitions.insert(
            DivisionId(1),
            DivisionDefinition {
                id: DivisionId(1),
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

        // アクティブな戦争を作成 (Country 1 vs Country 2, Goal: State 3)
        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
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
            occupied_states: HashSet::new(),
            status: WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        };
        war_registry.wars.insert(war.id, war);

        (
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
        )
    }

    #[test]
    fn test_frontline_generation_border_calculation() {
        let (state_registry, war_registry, _, _) = setup_test_environment();
        let war = war_registry.wars.get(&WarId(0)).unwrap();

        let (atk_front, def_front, pairs) = calculate_frontline_border(war, &state_registry);

        // State 2 (C1) と State 3 (C2) のみが国境ペアになる（Sea 5は除外）
        assert_eq!(atk_front, vec![StateId(2)]);
        assert_eq!(def_front, vec![StateId(3)]);
        assert_eq!(pairs, vec![(StateId(2), StateId(3))]);
    }

    #[test]
    fn test_frontline_lifecycle_creation_and_removal() {
        let (state_registry, war_registry, military_registry, mut frontline_registry) =
            setup_test_environment();

        // 前線の自動更新・生成
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        assert_eq!(frontline_registry.frontlines.len(), 1);
        let fl = frontline_registry.get_frontline_for_war(WarId(0)).unwrap();
        let fl_id = fl.frontline_id;

        // 重複生成せずIDが同一であることを確認
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        assert_eq!(frontline_registry.frontlines.len(), 1);
        assert_eq!(
            frontline_registry
                .get_frontline_for_war(WarId(0))
                .unwrap()
                .frontline_id,
            fl_id
        );

        // 削除テスト
        frontline_registry.remove_frontline(fl_id, &military_registry);
        assert!(frontline_registry.frontlines.is_empty());
    }

    #[test]
    fn test_army_assignment_and_validation() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        // C1の有効な陸軍を追加
        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);

        // C3 (無関係な国) の陸軍
        let a2 = ArmyUnit {
            id: ArmyId(1),
            owner: CountryId(3),
            manpower: 1000,
            ..military_registry.armies.get(&army_id).unwrap().clone()
        };
        let other_army_id = military_registry.add_army(a2);

        // 割当成功の検証
        assert!(
            frontline_registry
                .assign_army(
                    army_id,
                    fl_id,
                    CountryId(1),
                    &military_registry,
                    &war_registry
                )
                .is_ok()
        );
        assert_eq!(
            frontline_registry.army_frontline_map.get(&army_id),
            Some(&fl_id)
        );

        // 重複登録の防止
        assert!(
            frontline_registry
                .assign_army(
                    army_id,
                    fl_id,
                    CountryId(1),
                    &military_registry,
                    &war_registry
                )
                .is_ok()
        );
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(
            plan.assigned_army_ids
                .iter()
                .filter(|&&id| id == army_id)
                .count(),
            1
        );

        // 他国陸軍の割り当て拒否
        assert!(
            frontline_registry
                .assign_army(
                    other_army_id,
                    fl_id,
                    CountryId(1),
                    &military_registry,
                    &war_registry
                )
                .is_err()
        );

        // 解除
        assert!(
            frontline_registry
                .unassign_army(army_id, CountryId(1), &military_registry)
                .is_ok()
        );
        assert!(!frontline_registry.army_frontline_map.contains_key(&army_id));
    }

    /// P21-002回帰テスト: `unassign_army`は所有者と異なる`country_id`を渡すと拒否し、
    /// 割当状態を一切変更しない。UI経由(選択中陸軍は所有者不問で選択され得る)で
    /// 他国の前線割当を無断解除できてしまう不具合の修正確認。
    #[test]
    fn test_unassign_army_rejects_non_owner() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);
        frontline_registry
            .assign_army(
                army_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();

        // CountryId(1)の陸軍を、無関係な第三国CountryId(3)として解除しようとすると拒否される
        let result = frontline_registry.unassign_army(army_id, CountryId(3), &military_registry);
        assert!(result.is_err());
        assert_eq!(
            frontline_registry.army_frontline_map.get(&army_id),
            Some(&fl_id),
            "unauthorized unassign must not change the assignment"
        );
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert!(plan.assigned_army_ids.contains(&army_id));

        // 存在しない陸軍IDも拒否される
        let result_missing =
            frontline_registry.unassign_army(ArmyId(999), CountryId(1), &military_registry);
        assert!(result_missing.is_err());
    }

    #[test]
    fn test_evaluate_frontline_army_command_feasibility() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let frontline = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .clone();

        // 前線が存在しない場合
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[],
                CountryId(1),
                &military_registry,
                None
            ),
            FrontlineCommandFeasibility::NoActiveFrontline
        );

        // 前線はあるが陸軍未選択
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NoArmySelected
        );

        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);

        // 自国の有効な陸軍を選択中 → Ready
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[army_id],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::Ready
        );

        // 他国の陸軍を選択中(所有者不問で選択され得るため) → NotOwnArmy
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[army_id],
                CountryId(2),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NotOwnArmy
        );

        // 存在しない陸軍IDを選択中 → ArmyNotFound
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[ArmyId(999)],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::ArmyNotFound
        );

        // P21-003: 複数選択のうち1件でも実行可能ならReady
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[ArmyId(999), army_id],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::Ready
        );

        // P21-003: 全滅の場合は先頭(ArmyId昇順)の失敗理由を返す
        assert_eq!(
            evaluate_frontline_army_command_feasibility(
                &[army_id, ArmyId(999)],
                CountryId(2),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NotOwnArmy
        );
    }

    #[test]
    fn test_defensive_positioning_and_determinism() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        // C1の陸軍を追加 (State 1に待機中)
        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);
        let _ = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(1),
            &military_registry,
            &war_registry,
        );

        // Defend 命令へ設定して作戦実行
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
        );

        // 割当部隊が自国側前線 State 2 へ向かって移動を開始したことを確認
        let army = military_registry.armies.get(&army_id).unwrap();
        assert_eq!(army.status, ArmyStatus::Moving);
        assert_eq!(army.destination, Some(StateId(2)));
        assert_eq!(army.current_path, vec![StateId(2)]);
    }

    #[test]
    fn test_offensive_operations_objective_and_attack() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        // C1の陸軍を追加 (既に自国側前線 State 2 に待機中)
        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(2),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);
        let _ = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(1),
            &military_registry,
            &war_registry,
        );

        // Offensive 命令へ設定して作戦実行
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Offensive;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
        );

        // 待機中の部隊が隣接する戦争目標 State 3 へ進軍を開始したことを検証
        let army = military_registry.armies.get(&army_id).unwrap();
        assert_eq!(army.status, ArmyStatus::Moving);
        assert_eq!(army.destination, Some(StateId(3)));
    }

    #[test]
    fn test_manual_vs_frontline_priority() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        let a1 = ArmyUnit {
            id: ArmyId(0),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
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
            status: ArmyStatus::Idle,
            def_id: DivisionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let army_id = military_registry.add_army(a1);
        let _ = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(1),
            &military_registry,
            &war_registry,
        );

        // プレイヤーが手動で移動指示を出した想定
        if let Some(army) = military_registry.armies.get_mut(&army_id) {
            army.status = ArmyStatus::Moving;
            army.destination = Some(StateId(2));
            army.current_path = vec![StateId(2)];
        }
        // frontline_generated_movements に含まれていない＝手動移動

        // Defend 命令を出しても、手動移動中の部隊は上書きされない
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
        );

        let army = military_registry.armies.get(&army_id).unwrap();
        assert_eq!(army.status, ArmyStatus::Moving);
        assert_eq!(army.destination, Some(StateId(2)));

        // 停止命令 (Stopped) を出しても、手動移動経路は解除されない
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Stopped;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
        );

        let army_after_stop = military_registry.armies.get(&army_id).unwrap();
        assert_eq!(army_after_stop.status, ArmyStatus::Moving);
        assert_eq!(army_after_stop.destination, Some(StateId(2)));
    }
}
