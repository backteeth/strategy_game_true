use crate::app::time::DayChangedMessage;
use crate::common::{ArmyId, CountryId, DivisionId, FrontlineId, StateId, WarId};
use crate::military::army::ArmyRegistry;
use crate::military::data::{DivisionStatus, MilitaryRegistry};
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
    pub assigned_division_ids: Vec<DivisionId>,
    /// P21-005: この前線・国家へ割り当てられたArmy(編成)。`assigned_division_ids`とは
    /// 完全に独立した参照であり、`process_defensive_plan`/`process_offensive_plan`/
    /// `process_stopped_plan`はこのフィールドを一切読まない(=Army割当だけでは
    /// 自動移動が一切発生しない)。旧セーブにフィールド自体が存在しない場合は
    /// 空のVecとして復元する。
    #[serde(default)]
    pub assigned_army_ids: Vec<ArmyId>,
    /// P21-007: 攻勢線(計画データのみ)。`process_offensive_plan`・自動移動・自動攻撃・
    /// 戦闘開始のいずれからも一切参照されない(このラウンドでは接続しない)。
    /// StateId昇順・重複なしで保持し、1つのPlanにつき最大1本。`Some(空Vec)`は無効な
    /// 状態として扱う(未設定は常に`None`。`FrontlineRegistry::set_offensive_line`/
    /// `clear_offensive_line`だけがこのフィールドを書き換える)。旧セーブにフィールド
    /// 自体が存在しない場合は`#[serde(default)]`によりNoneとして復元する。
    #[serde(default)]
    pub offensive_line_region_ids: Option<Vec<StateId>>,
}

impl FrontlinePlan {
    pub fn new(frontline_id: FrontlineId, commanding_country_id: CountryId) -> Self {
        Self {
            frontline_id,
            commanding_country_id,
            stance: FrontlineStance::Stopped,
            objective_region_id: None,
            assigned_division_ids: Vec::new(),
            assigned_army_ids: Vec::new(),
            offensive_line_region_ids: None,
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
    /// DivisionId -> FrontlineId のマッピング（1陸軍は1前線のみ所属）
    pub division_frontline_map: HashMap<DivisionId, FrontlineId>,
    /// 前線によって自動生成された移動命令を実行中の陸軍セット
    pub frontline_generated_movements: HashSet<DivisionId>,
    /// P21-005: ArmyId -> FrontlineId のマッピング（1つのArmyは同時に最大1つの前線のみ所属）。
    /// `division_frontline_map`と対になる正規情報で、`plans[..].assigned_army_ids`と
    /// `assign_army`/`unassign_army`/`remove_frontline`/`sanitize_army_references`が
    /// 常に一括で更新する(独立に毎フレーム同期する設計ではない)。
    #[serde(default)]
    pub army_frontline_map: HashMap<ArmyId, FrontlineId>,
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
    pub fn assign_division(
        &mut self,
        division_id: DivisionId,
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

        let division = military_registry
            .divisions
            .get(&division_id)
            .ok_or("Division not found")?;
        if division.owner != country_id {
            return Err("Division belongs to a different country");
        }
        if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
            return Err("Division is destroyed or has 0 manpower");
        }

        // 他の前線からの割当解除
        if let Some(old_fl_id) = self.division_frontline_map.remove(&division_id)
            && let Some(old_plan) = self.plans.get_mut(&(old_fl_id, country_id))
        {
            old_plan
                .assigned_division_ids
                .retain(|&id| id != division_id);
        }

        // 対象Planの取得または作成
        let plan = self
            .plans
            .entry((frontline_id, country_id))
            .or_insert_with(|| FrontlinePlan::new(frontline_id, country_id));

        if !plan.assigned_division_ids.contains(&division_id) {
            plan.assigned_division_ids.push(division_id);
            plan.assigned_division_ids.sort_by_key(|id| id.0);
        }

        self.division_frontline_map
            .insert(division_id, frontline_id);
        Ok(())
    }

    /// 陸軍の前線割り当てを解除する
    ///
    /// `country_id`の陸軍であることを検証してから解除する。P21-002以前は所有者検証が
    /// 存在せず、選択中の陸軍(所有者不問で選択可能、`map::division_selection`参照)が
    /// 敵国や第三国の陸軍であっても無条件に前線割り当てを解除できてしまっていた。
    pub fn unassign_division(
        &mut self,
        division_id: DivisionId,
        country_id: CountryId,
        military_registry: &MilitaryRegistry,
    ) -> Result<(), &'static str> {
        let division = military_registry
            .divisions
            .get(&division_id)
            .ok_or("Division not found")?;
        if division.owner != country_id {
            return Err("Division belongs to a different country");
        }

        if let Some(fl_id) = self.division_frontline_map.remove(&division_id) {
            for plan in self.plans.values_mut() {
                if plan.frontline_id == fl_id {
                    plan.assigned_division_ids.retain(|&id| id != division_id);
                }
            }
        }
        self.frontline_generated_movements.remove(&division_id);
        Ok(())
    }

    /// 前線の全陸軍割当を解除する
    pub fn unassign_all_divisions_for_plan(
        &mut self,
        frontline_id: FrontlineId,
        country_id: CountryId,
    ) {
        if let Some(plan) = self.plans.get_mut(&(frontline_id, country_id)) {
            for division_id in plan.assigned_division_ids.drain(..) {
                self.division_frontline_map.remove(&division_id);
                self.frontline_generated_movements.remove(&division_id);
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
            plan.assigned_division_ids.retain(|&division_id| {
                if let Some(division) = military_registry.divisions.get(&division_id) {
                    division.manpower > 0
                        && division.status != DivisionStatus::Destroyed
                        && division.owner == plan.commanding_country_id
                } else {
                    false
                }
            });
        }

        // division_frontline_map の整理
        self.division_frontline_map.retain(|division_id, fl_id| {
            if !self.frontlines.contains_key(fl_id) {
                return false;
            }
            if let Some(division) = military_registry.divisions.get(division_id) {
                division.manpower > 0 && division.status != DivisionStatus::Destroyed
            } else {
                false
            }
        });

        // frontline_generated_movements の整理
        self.frontline_generated_movements
            .retain(|division_id| military_registry.divisions.contains_key(division_id));
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
                for division_id in plan.assigned_division_ids {
                    self.division_frontline_map.remove(&division_id);
                    self.frontline_generated_movements.remove(&division_id);
                }
                // P21-005: このPlanが保持していたArmy割当も一括で清掃する。
                for army_id in plan.assigned_army_ids {
                    self.army_frontline_map.remove(&army_id);
                }
            }
        }

        // frontline_generated_movementsの解除と、該当陸軍の前線移動停止
        let _ = military_registry;
    }

    /// P21-005: Armyを前線へ割り当てる。`assign_division`と同じ検証方針(前線存在・War
    /// Active・国が前線参加国・Army所有者一致)に加え、Armyの存在確認を行う。
    /// 割当は`plans[..].assigned_army_ids`と`army_frontline_map`の両方を単一操作として
    /// 更新するのみで、`Division`側のいかなるフィールド(current_state/destination/
    /// current_path/target_state/movement_progress/status/combat_id)も一切変更しない。
    /// 移動Message発行・`frontline_generated_movements`への追加・戦闘開始も行わない。
    pub fn assign_army(
        &mut self,
        army_id: ArmyId,
        frontline_id: FrontlineId,
        country_id: CountryId,
        army_registry: &ArmyRegistry,
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

        let army = army_registry.armies.get(&army_id).ok_or("Army not found")?;
        if army.owner != country_id {
            return Err("Army belongs to a different country");
        }

        // 他の前線からの割当解除(旧Planから外す。旧前線が既に無効でも
        // army_frontline_mapに残っていれば同様に処理する)
        if let Some(old_fl_id) = self.army_frontline_map.remove(&army_id)
            && let Some(old_plan) = self.plans.get_mut(&(old_fl_id, country_id))
        {
            old_plan.assigned_army_ids.retain(|&id| id != army_id);
        }

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

    /// P21-005: Armyの前線割当を解除する。`country_id`がArmyの所有者であることを
    /// 検証してから解除する(`unassign_division`と同じ所有者検証パターン)。
    /// 解除してもDivisionの現在地・移動先・経路・戦闘状態は一切変更しない。
    pub fn unassign_army(
        &mut self,
        army_id: ArmyId,
        country_id: CountryId,
        army_registry: &ArmyRegistry,
    ) -> Result<(), &'static str> {
        let army = army_registry.armies.get(&army_id).ok_or("Army not found")?;
        if army.owner != country_id {
            return Err("Army belongs to a different country");
        }

        if let Some(fl_id) = self.army_frontline_map.remove(&army_id)
            && let Some(plan) = self.plans.get_mut(&(fl_id, country_id))
        {
            plan.assigned_army_ids.retain(|&id| id != army_id);
        }
        Ok(())
    }

    /// P21-005: 指定Armyが現在割り当てられている前線を取得する。
    pub fn frontline_for_army(&self, army_id: ArmyId) -> Option<FrontlineId> {
        self.army_frontline_map.get(&army_id).copied()
    }

    /// P21-005: `owner`が割当可能な(War Active・自国が参加国側の)前線一覧を
    /// FrontlineId昇順で決定的に返す。UIのボタン活性判定・前線設定モードの
    /// ハイライト対象決定・モードの自動キャンセル判定(対象Warが消滅済みか)の
    /// いずれからも共通で使う唯一の判定経路。
    pub fn assignable_frontlines_for_army(
        &self,
        owner: CountryId,
        war_registry: &WarRegistry,
    ) -> Vec<FrontlineId> {
        let mut ids: Vec<FrontlineId> = self
            .frontlines
            .values()
            .filter(|fl| fl.attacker_country_id == owner || fl.defender_country_id == owner)
            .filter(|fl| {
                war_registry
                    .wars
                    .get(&fl.war_id)
                    .map(|w| w.status == WarStatus::Active)
                    .unwrap_or(false)
            })
            .map(|fl| fl.frontline_id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// P21-005: 消滅・解散済みArmyの参照を全Planと`army_frontline_map`から整理する
    /// (`sanitize_references`のArmy版)。Army自体はDivisionと異なり日次更新に限らず
    /// UIボタン操作で即座に消滅しうるため(解散・除外による自動解散)、呼び出し側は
    /// `ArmyRegistry`が変化した毎フレーム(`is_changed()`ガード付き)これを呼ぶ。
    pub fn sanitize_army_references(&mut self, army_registry: &ArmyRegistry) {
        for plan in self.plans.values_mut() {
            let commanding_country_id = plan.commanding_country_id;
            plan.assigned_army_ids.retain(|&army_id| {
                army_registry
                    .armies
                    .get(&army_id)
                    .map(|a| a.owner == commanding_country_id)
                    .unwrap_or(false)
            });
        }

        let frontlines = &self.frontlines;
        self.army_frontline_map.retain(|army_id, fl_id| {
            frontlines.contains_key(fl_id) && army_registry.armies.contains_key(army_id)
        });
    }

    /// P21-007: 攻勢線(計画データのみ)を設定する。全条件を検証してから一括で書き込み、
    /// 1件でも不正なら`plan.offensive_line_region_ids`を一切変更しない。
    ///
    /// 検証順序: 前線存在 → War Active → `country_id`が前線参加国 → `region_ids`が
    /// 非空 → 各IDについて(前方から見た)重複無し・州として実在・陸地・
    /// 「この前線のこの戦争における敵国」が支配中(`StateData::controller()`、
    /// `process_defensive_plan`/`process_offensive_plan`の敵国判定と同じ計算式) →
    /// 2件以上の場合は`StateData.neighbors`を対象IDの集合に限定したグラフ上で
    /// 連結しているか。成功時のみStateId昇順へソートして`Some(..)`を書き込む
    /// (`Some(空Vec)`は仕様上無効な状態であり、ここでは絶対に書き込まない設計になる:
    /// `region_ids.is_empty()`は事前に拒否するため)。
    ///
    /// このメソッド自身は`process_offensive_plan`・自動移動・自動攻撃・戦闘開始の
    /// いずれからも呼ばれない(呼び出しはUI層の確定操作のみを起点とする)。
    pub fn set_offensive_line(
        &mut self,
        frontline_id: FrontlineId,
        country_id: CountryId,
        region_ids: &[StateId],
        state_registry: &StateRegistry,
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

        if region_ids.is_empty() {
            return Err("Offensive line must not be empty");
        }

        let enemy_id = if country_id == frontline.attacker_country_id {
            frontline.defender_country_id
        } else {
            frontline.attacker_country_id
        };

        let mut seen: HashSet<StateId> = HashSet::new();
        for &region_id in region_ids {
            if !seen.insert(region_id) {
                return Err("Offensive line contains a duplicate region");
            }
            let state = state_registry
                .get(region_id)
                .ok_or("Offensive line references an unknown region")?;
            if state.is_sea {
                return Err("Offensive line cannot include sea regions");
            }
            if state.controller() != enemy_id {
                return Err(
                    "Offensive line must consist of regions controlled by this frontline's enemy",
                );
            }
        }

        if region_ids.len() > 1 {
            let region_set: HashSet<StateId> = region_ids.iter().copied().collect();
            let mut visited: HashSet<StateId> = HashSet::new();
            let mut stack = vec![region_ids[0]];
            visited.insert(region_ids[0]);
            while let Some(current) = stack.pop() {
                if let Some(state) = state_registry.get(current) {
                    for &neighbor in &state.neighbors {
                        if region_set.contains(&neighbor) && visited.insert(neighbor) {
                            stack.push(neighbor);
                        }
                    }
                }
            }
            if visited.len() != region_set.len() {
                return Err("Offensive line regions must form a single connected component");
            }
        }

        let mut sorted_ids: Vec<StateId> = region_ids.to_vec();
        sorted_ids.sort_by_key(|s| s.0);

        let plan = self
            .plans
            .entry((frontline_id, country_id))
            .or_insert_with(|| FrontlinePlan::new(frontline_id, country_id));
        plan.offensive_line_region_ids = Some(sorted_ids);
        Ok(())
    }

    /// P21-007: 攻勢線を即座に解除する(確認ダイアログ不要の仕様に合わせ、Planが
    /// 存在すればフィールドを`None`へ戻すだけ)。Planが存在しない場合は何もしない。
    pub fn clear_offensive_line(&mut self, frontline_id: FrontlineId, country_id: CountryId) {
        if let Some(plan) = self.plans.get_mut(&(frontline_id, country_id)) {
            plan.offensive_line_region_ids = None;
        }
    }
}

/// P21-002: 前線割当/解除ボタンが実行可能かを副作用なしで判定する結果。
/// UIの表示更新(ボタン活性/非活性)とクリックハンドラの双方から利用する
/// (`military::recruitment::RecruitFeasibility`と同型のパターン)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontlineCommandFeasibility {
    Ready,
    NoActiveFrontline,
    NoDivisionSelected,
    DivisionNotFound,
    NotOwnDivision,
    DivisionDestroyed,
}

impl FrontlineCommandFeasibility {
    pub fn is_ready(self) -> bool {
        matches!(self, FrontlineCommandFeasibility::Ready)
    }
}

/// 選択中の陸軍を前線へ割当/解除する操作が可能かを判定する。
///
/// `selected_division_id`は所有者を問わず選択され得る(`map::division_selection::handle_division_selection`
/// 参照)ため、ここで所有者を必ず再検証する。`assign_division`/`unassign_division`実行時にも
/// 同じ検証を行うため、この関数の結果は表示専用であり実行の可否を最終的に保証するのは
/// 実行系側(`assign_division`/`unassign_division`自身)である。
fn evaluate_single_division_command_feasibility(
    division_id: DivisionId,
    player_cid: CountryId,
    military_registry: &MilitaryRegistry,
) -> FrontlineCommandFeasibility {
    let Some(division) = military_registry.divisions.get(&division_id) else {
        return FrontlineCommandFeasibility::DivisionNotFound;
    };
    if division.owner != player_cid {
        return FrontlineCommandFeasibility::NotOwnDivision;
    }
    if division.manpower == 0 || division.status == DivisionStatus::Destroyed {
        return FrontlineCommandFeasibility::DivisionDestroyed;
    }
    FrontlineCommandFeasibility::Ready
}

/// P21-003: 複数選択に対応。`selected_division_ids`のうち1件でも実行可能ならReadyを返す
/// (複数選択時は一部の陸軍だけでも割当/解除が成立すれば十分なため)。全滅なら、
/// `selected_division_ids`をDivisionId昇順に評価した最初の失敗理由を返す(呼び出し側が
/// ソート済みの配列を渡すことで結果を決定的にする)。
pub fn evaluate_frontline_division_command_feasibility(
    selected_division_ids: &[DivisionId],
    player_cid: CountryId,
    military_registry: &MilitaryRegistry,
    frontline: Option<&Frontline>,
) -> FrontlineCommandFeasibility {
    if frontline.is_none() {
        return FrontlineCommandFeasibility::NoActiveFrontline;
    }
    if selected_division_ids.is_empty() {
        return FrontlineCommandFeasibility::NoDivisionSelected;
    }

    let mut first_failure = FrontlineCommandFeasibility::NoDivisionSelected;
    for &division_id in selected_division_ids {
        let result = evaluate_single_division_command_feasibility(
            division_id,
            player_cid,
            military_registry,
        );
        if result == FrontlineCommandFeasibility::Ready {
            return FrontlineCommandFeasibility::Ready;
        }
        if first_failure == FrontlineCommandFeasibility::NoDivisionSelected {
            first_failure = result;
        }
    }
    first_failure
}

/// P21-005: Army「前線を設定」ボタンが実行可能かを副作用なしで判定する結果。
/// `FrontlineCommandFeasibility`と同型のパターン(表示専用。実行の可否を最終的に
/// 保証するのは`assign_army`自身)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmyFrontlineAssignFeasibility {
    Ready,
    NoArmySelected,
    ArmyNotFound,
    NotOwnArmy,
    NoAssignableFrontline,
}

impl ArmyFrontlineAssignFeasibility {
    pub fn is_ready(self) -> bool {
        matches!(self, ArmyFrontlineAssignFeasibility::Ready)
    }
}

/// 選択中のArmyに対し「前線を設定」操作(前線選択モードへ入る)が可能かを判定する。
pub fn evaluate_army_frontline_assign_feasibility(
    selected_army_id: Option<ArmyId>,
    player_cid: CountryId,
    army_registry: &ArmyRegistry,
    frontline_registry: &FrontlineRegistry,
    war_registry: &WarRegistry,
) -> ArmyFrontlineAssignFeasibility {
    let Some(army_id) = selected_army_id else {
        return ArmyFrontlineAssignFeasibility::NoArmySelected;
    };
    let Some(army) = army_registry.armies.get(&army_id) else {
        return ArmyFrontlineAssignFeasibility::ArmyNotFound;
    };
    if army.owner != player_cid {
        return ArmyFrontlineAssignFeasibility::NotOwnArmy;
    }
    if frontline_registry
        .assignable_frontlines_for_army(army.owner, war_registry)
        .is_empty()
    {
        return ArmyFrontlineAssignFeasibility::NoAssignableFrontline;
    }
    ArmyFrontlineAssignFeasibility::Ready
}

/// 前線境界を実効支配地域から決定的に計算する
pub fn calculate_frontline_border(
    war: &War,
    state_registry: &StateRegistry,
) -> (Vec<StateId>, Vec<StateId>, Vec<(StateId, StateId)>) {
    // P21-016: 前線境界の描画は既存どおり明示的な代表国
    // (primary_attacker/primary_defender)を基準とした二国間の概念のまま据え置く
    // (多国間参加者全体を結ぶ前線メッシュ描画は本タスクの対象外)。
    let attacker = war.primary_attacker_id();
    let defender = war.primary_defender_id();

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
    let attacker = war.primary_attacker_id();
    let defender = war.primary_defender_id();

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
    let attacker = war.primary_attacker_id();
    let defender = war.primary_defender_id();

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

/// P21-006: 指定Divisionが現在従うべき「有効な前線」を解決する。
///
/// 優先順位: Divisionの直接前線割当(`division_frontline_map`)が常に最優先。直接割当が
/// 無いDivisionだけが、所属Army(`division_army_map`経由)の前線割当(`army_frontline_map`)を
/// 継承する。派生状態を一切持たない純粋関数で、`division_frontline_map`/
/// `assigned_division_ids`を複製・書換えすることはない。全DivisionやArmyを走査せず、
/// 指定された1件のDivisionIdに対してO(1)のHashMap参照のみで解決する。
pub fn resolve_effective_frontline_for_division(
    division_id: DivisionId,
    frontline_registry: &FrontlineRegistry,
    army_registry: &ArmyRegistry,
) -> Option<FrontlineId> {
    if let Some(&fl_id) = frontline_registry.division_frontline_map.get(&division_id) {
        return Some(fl_id);
    }
    let army_id = army_registry.division_army_map.get(&division_id)?;
    frontline_registry.army_frontline_map.get(army_id).copied()
}

/// P21-006: 特定の(Frontline, Country)Planについて、実際に防御配置処理の対象となる
/// DivisionIdの集合を実行時に解決する。
///
/// `plan.assigned_division_ids`(直接割当)に加え、`plan.assigned_army_ids`(このplanへ
/// 割り当てられたArmy)の`member_division_ids`のうち直接割当を持たないものを合流させる。
/// 直接割当を持つDivisionはArmy経由では二重に処理されない
/// (`resolve_effective_frontline_for_division`と同じ優先順位規則)。
/// `plan.assigned_army_ids`は既にこのplan専用に絞り込まれているため、全Army・全Divisionを
/// 走査することはない(`assign_army`が国・前線参加要件を割当時に検証済み)。
/// 結果はDivisionId昇順・重複無しで返し、HashMapの反復順には一切依存しない。
pub fn resolve_effective_division_ids_for_plan(
    plan: &FrontlinePlan,
    army_registry: &ArmyRegistry,
    division_frontline_map: &HashMap<DivisionId, FrontlineId>,
) -> Vec<DivisionId> {
    let mut ids: Vec<DivisionId> = plan.assigned_division_ids.clone();

    let mut army_ids: Vec<ArmyId> = plan.assigned_army_ids.clone();
    army_ids.sort_by_key(|id| id.0);

    for army_id in army_ids {
        let Some(army) = army_registry.armies.get(&army_id) else {
            continue;
        };
        if army.owner != plan.commanding_country_id {
            continue;
        }
        for &division_id in &army.member_division_ids {
            // 直接割当が優先。既にdivision_frontline_mapに登録済みのDivisionは
            // (このplan向けであれ他のFrontline向けであれ)Army経由では処理しない。
            if division_frontline_map.contains_key(&division_id) {
                continue;
            }
            ids.push(division_id);
        }
    }

    ids.sort_by_key(|id| id.0);
    ids.dedup();
    ids
}

/// P21-008: OffensiveからDefendへ実際に切り替えた際、攻勢線経由で発行された
/// (前線国境地域の外、すなわち敵領を指す)在途中の移動命令を明示的に取り消す。
///
/// `process_defensive_plan`自体は汎用の前線再配置ロジックであり、Defend姿勢への
/// dispatchだけでなく、Offensive姿勢継続中の内部前段としても毎日呼ばれる。もし
/// `process_defensive_plan`自身に「現在地が既に配置先と一致するなら取り消す」という
/// ロジックを足すと、Offensive姿勢が継続している間の在途中の攻撃移動(まだcurrent_state
/// が動いていない=前線国境地域のまま、destinationだけ敵領を指している状態)まで
/// 誤って毎日取り消してしまう(実装中に発見・修正した回帰)。そのため取消は、
/// 実際にDefendへdispatchされたこの経路だけに限定し、`process_defensive_plan`自体は
/// 一切変更しない。前線国境地域内を指す移動(通常の防御再配置)はそのまま
/// `process_defensive_plan`に委ねる(取り消さない)。
fn cancel_stale_offensive_movements_before_defend(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    army_registry: &ArmyRegistry,
) {
    let country_id = plan.commanding_country_id;
    let front_regions = if country_id == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    let division_ids = resolve_effective_division_ids_for_plan(
        plan,
        army_registry,
        &frontline_registry.division_frontline_map,
    );

    for division_id in division_ids {
        if !frontline_registry
            .frontline_generated_movements
            .contains(&division_id)
        {
            continue;
        }
        let Some(division) = military_registry.divisions.get(&division_id) else {
            continue;
        };
        if division.status != DivisionStatus::Moving {
            continue;
        }
        let target = division.destination.unwrap_or(division.current_state);
        if front_regions.contains(&target) {
            // 通常の防御再配置による移動先。取り消さず`process_defensive_plan`に委ねる。
            continue;
        }
        if let Some(a_mut) = military_registry.divisions.get_mut(&division_id) {
            a_mut.destination = None;
            a_mut.current_path.clear();
            a_mut.target_state = None;
            a_mut.movement_progress = 0.0;
            a_mut.status = DivisionStatus::Idle;
        }
        frontline_registry
            .frontline_generated_movements
            .remove(&division_id);
    }
}

/// 防御配置処理 (Defend Stance)
pub fn process_defensive_plan(
    frontline: &Frontline,
    plan: &FrontlinePlan,
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    _war_registry: &WarRegistry,
    frontline_registry: &mut FrontlineRegistry,
    army_registry: &ArmyRegistry,
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

    // P21-006: 直接割当(plan.assigned_division_ids)に加え、直接割当のないDivisionのうち
    // 所属Armyがこのplanへ割り当てられているものを実行時に解決して合流させる
    // (division_frontline_map/assigned_division_idsを複製・書換えしない)。
    let division_ids = resolve_effective_division_ids_for_plan(
        plan,
        army_registry,
        &frontline_registry.division_frontline_map,
    );

    if front_regions.is_empty() || division_ids.is_empty() {
        return;
    }

    // 1. 各前線地域に配置中・移動目的地中の割当陸軍数を集計
    let mut region_counts: HashMap<StateId, usize> =
        front_regions.iter().map(|&r| (r, 0)).collect();

    for &division_id in &division_ids {
        if let Some(division) = military_registry.divisions.get(&division_id) {
            let target = division.destination.unwrap_or(division.current_state);
            if region_counts.contains_key(&target) {
                *region_counts.get_mut(&target).unwrap() += 1;
            }
        }
    }

    // 2. 陸軍をDivisionId順で処理し、最適な配置先へ移動指示(resolve関数が既にソート済み)
    for division_id in division_ids {
        let division = match military_registry.divisions.get(&division_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        // 命令受給可能か判定
        if division.manpower == 0
            || division.status == DivisionStatus::Fighting
            || division.status == DivisionStatus::Retreating
            || division.status == DivisionStatus::Destroyed
        {
            continue;
        }

        // 手動移動中は上書きしない（frontline_generated_movementsに含まれず、かつMovingの場合はスキップ）
        if division.status == DivisionStatus::Moving
            && !frontline_registry
                .frontline_generated_movements
                .contains(&division_id)
        {
            continue;
        }

        // 既に前線地域におり、待機中の場合
        if front_regions.contains(&division.current_state)
            && division.status == DivisionStatus::Idle
        {
            // 現在地がまだ配置バランス上問題なければ維持
            continue;
        }

        // 移動中の場合はカウントを一旦仮減算して再配置決定
        let curr_target = division.destination.unwrap_or(division.current_state);
        if let Some(count) = region_counts.get_mut(&curr_target) {
            *count = count.saturating_sub(1);
        }

        // 最適配置先の選定
        // 条件: 1. 配置数最小 -> 2. 距離/ステップ数 -> 3. StateId最小
        let mut candidates: Vec<(StateId, usize, usize)> = Vec::new();

        for &region_id in front_regions {
            let count = *region_counts.get(&region_id).unwrap_or(&0);
            if let Some(path) = find_path(
                division.current_state,
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

            if division.current_state != best_region
                && let Some(path) = find_path(
                    division.current_state,
                    best_region,
                    state_registry,
                    &[country_id],
                    &[enemy_id],
                )
                && !path.is_empty()
                && let Some(a_mut) = military_registry.divisions.get_mut(&division_id)
            {
                a_mut.destination = Some(best_region);
                a_mut.current_path = path;
                a_mut.target_state = None;
                a_mut.status = DivisionStatus::Moving;
                a_mut.movement_progress = 0.0;
                frontline_registry
                    .frontline_generated_movements
                    .insert(division_id);
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
    army_registry: &ArmyRegistry,
) {
    let country_id = plan.commanding_country_id;
    let enemy_id = if country_id == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    // P21-008: 攻勢線が設定されている場合は、objective_region_idベースの単一目標処理
    // (自動決定・達成時Defend自動遷移・直接割当Divisionのみの1ホップ隣接攻撃)を
    // 一切実行せず、攻勢線の未確保State集合を目標とする別経路
    // (`process_offensive_line_attack`)へ完全に分岐する。攻勢線がなければ、以降は
    // P21-006までの既存処理を一切変更しない。
    if let Some(line) = plan.offensive_line_region_ids.clone() {
        // まず自国側前線へ陸軍を分散配置(既存のOffensive処理と同じ、Army経由分も合流)。
        process_defensive_plan(
            frontline,
            plan,
            state_registry,
            military_registry,
            war_registry,
            frontline_registry,
            army_registry,
        );
        process_offensive_line_attack(
            plan,
            &line,
            state_registry,
            military_registry,
            frontline_registry,
            army_registry,
            country_id,
            enemy_id,
        );
        return;
    }

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
            army_registry,
        );
        return;
    }

    // まず自国側前線へ陸軍を分散配置（移動中・待機中へ）。P21-006: この呼び出し経由で
    // Army経由のDivisionも既存の防御配置(自国側前線への分散)に合流する。
    process_defensive_plan(
        frontline,
        plan,
        state_registry,
        military_registry,
        war_registry,
        frontline_registry,
        army_registry,
    );

    // 待機中 (Idle) で自国側前線地域にいる陸軍から攻撃命令を発行
    let front_regions = if country_id == frontline.attacker_country_id {
        &frontline.attacker_front_regions
    } else {
        &frontline.defender_front_regions
    };

    // P21-006: 攻撃命令の発行対象は意図的に直接割当(assigned_division_ids)のみに限定する
    // (Army経由のDivisionを攻勢の自動攻撃対象へ含めない。P21-006のスコープ外)。
    // 防御配置(前線への分散配置)は上のprocess_defensive_plan呼び出しで既にArmy経由分も
    // 処理済みだが、そこから先の「隣接する敵州への攻撃」はこのplan.assigned_division_ids
    // だけを見る既存の経路のまま変更しない。
    let mut division_ids = plan.assigned_division_ids.clone();
    division_ids.sort_by_key(|id| id.0);

    let mut daily_target_counts: HashMap<StateId, usize> = HashMap::new();

    for division_id in division_ids {
        let division = match military_registry.divisions.get(&division_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        if division.status != DivisionStatus::Idle || division.manpower == 0 {
            continue;
        }

        if !front_regions.contains(&division.current_state) {
            continue;
        }

        let curr_state = match state_registry.get(division.current_state) {
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

            if let Some(a_mut) = military_registry.divisions.get_mut(&division_id) {
                a_mut.destination = Some(best_target);
                a_mut.current_path = vec![best_target];
                a_mut.target_state = None;
                a_mut.status = DivisionStatus::Moving;
                a_mut.movement_progress = 0.0;
                frontline_registry
                    .frontline_generated_movements
                    .insert(division_id);
            }
        }
    }
}

/// P21-008: 攻勢線のうち、まだ自国支配下にない(=未確保の)StateIdだけをStateId昇順で返す
/// 純粋関数。`line`は既に`FrontlineRegistry::set_offensive_line`によってStateId昇順・
/// 重複なしで保持されているため、フィルタ後もその順序を維持する。空を返す場合は
/// 「攻勢線を全て確保済み」を意味する。
pub fn uncaptured_offensive_line_regions(
    line: &[StateId],
    state_registry: &StateRegistry,
    country_id: CountryId,
) -> Vec<StateId> {
    line.iter()
        .copied()
        .filter(|&sid| {
            state_registry
                .get(sid)
                .is_some_and(|s| s.controller() != country_id)
        })
        .collect()
}

/// P21-008: 攻勢線(計画データ)を実際の攻撃目標として、有効Division(P21-006の
/// `resolve_effective_division_ids_for_plan`、直接割当優先・重複排除済み)へ移動命令を
/// 発行する。`process_offensive_plan`のobjective_region_idベース処理とは完全に独立した
/// 経路であり、こちらは攻勢線が設定されている場合にのみ呼ばれる。
///
/// 既存の直接攻撃ループ(隣接1ホップ攻撃)と同じ受給条件(Idle・manpower>0)、同じ
/// tie-break規則(経路長→本日の目標別割当数→StateId昇順)を踏襲するが、目標は
/// 「隣接する敵州」ではなく「攻勢線のうち未確保のState」の集合であり、複数ホップの
/// フルパス(`find_path`の結果全体)を発行する点が異なる(`process_defensive_plan`と
/// 同じパス発行スタイル)。到達可能な未確保Stateが1件もないDivisionには何も発行しない。
/// 全State確保済みなら、この関数は何も行わずに返る(Offensive姿勢は維持され、
/// Defendへの自動遷移はしない — 攻勢線には「到達済み」という独自の終端状態がある)。
#[allow(clippy::too_many_arguments)]
fn process_offensive_line_attack(
    plan: &FrontlinePlan,
    line: &[StateId],
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    army_registry: &ArmyRegistry,
    country_id: CountryId,
    enemy_id: CountryId,
) {
    let uncaptured = uncaptured_offensive_line_regions(line, state_registry, country_id);
    if uncaptured.is_empty() {
        return;
    }

    let division_ids = resolve_effective_division_ids_for_plan(
        plan,
        army_registry,
        &frontline_registry.division_frontline_map,
    );
    if division_ids.is_empty() {
        return;
    }

    let mut daily_target_counts: HashMap<StateId, usize> = HashMap::new();

    for division_id in division_ids {
        let division = match military_registry.divisions.get(&division_id) {
            Some(a) => a.clone(),
            None => continue,
        };

        if division.status != DivisionStatus::Idle || division.manpower == 0 {
            continue;
        }

        // 到達可能な未確保目標を、経路長→本日の目標別割当数→StateId昇順で決定的に選ぶ。
        let mut candidates: Vec<(StateId, usize, usize)> = Vec::new();
        for &target in &uncaptured {
            if let Some(path) = find_path(
                division.current_state,
                target,
                state_registry,
                &[country_id],
                &[enemy_id],
            ) {
                let cnt = *daily_target_counts.get(&target).unwrap_or(&0);
                candidates.push((target, path.len(), cnt));
            }
        }
        if candidates.is_empty() {
            // 到達不能: このDivisionには不正な移動を発行しない。
            continue;
        }
        candidates.sort_by(|a, b| {
            a.1.cmp(&b.1) // 経路長最小
                .then_with(|| a.2.cmp(&b.2)) // 本日の割当数最小
                .then_with(|| a.0.0.cmp(&b.0.0)) // StateId最小
        });

        let (best_target, _, _) = candidates[0];
        if division.current_state == best_target {
            continue;
        }

        if let Some(path) = find_path(
            division.current_state,
            best_target,
            state_registry,
            &[country_id],
            &[enemy_id],
        ) && !path.is_empty()
        {
            *daily_target_counts.entry(best_target).or_default() += 1;
            if let Some(a_mut) = military_registry.divisions.get_mut(&division_id) {
                a_mut.destination = Some(best_target);
                a_mut.current_path = path;
                a_mut.target_state = None;
                a_mut.status = DivisionStatus::Moving;
                a_mut.movement_progress = 0.0;
                frontline_registry
                    .frontline_generated_movements
                    .insert(division_id);
            }
        }
    }
}

/// P21-008: 攻勢線の現在の進捗状態。UI表示専用の副作用なし判定
/// (`ArmyFrontlineAssignFeasibility`と同型のパターン)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffensiveLineProgress {
    /// 攻勢線が設定されていない。
    NotSet,
    /// 攻勢線は設定済みだが、現在の姿勢がOffensiveではない(準備中)。
    Preparing,
    /// Offensive姿勢で、未確保Stateが残っており、少なくとも1体が到達可能。
    InProgress,
    /// 攻勢線の全Stateを確保済み。
    Reached,
    /// Offensive姿勢で未確保Stateが残っているが、有効Divisionのいずれからも
    /// 到達可能な未確保Stateが1つもない。
    NoReachableTargets,
}

/// P21-008: `OffensiveLineProgress`を判定する。副作用なし・Worldを変更しない。
/// UIの表示更新だけでなく、テストからも同じ判定経路を利用できるようにする。
pub fn compute_offensive_line_progress(
    plan: &FrontlinePlan,
    frontline: &Frontline,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    army_registry: &ArmyRegistry,
    frontline_registry: &FrontlineRegistry,
) -> OffensiveLineProgress {
    let Some(line) = &plan.offensive_line_region_ids else {
        return OffensiveLineProgress::NotSet;
    };

    let country_id = plan.commanding_country_id;
    let uncaptured = uncaptured_offensive_line_regions(line, state_registry, country_id);
    if uncaptured.is_empty() {
        return OffensiveLineProgress::Reached;
    }

    if plan.stance != FrontlineStance::Offensive {
        return OffensiveLineProgress::Preparing;
    }

    let enemy_id = if country_id == frontline.attacker_country_id {
        frontline.defender_country_id
    } else {
        frontline.attacker_country_id
    };

    let division_ids = resolve_effective_division_ids_for_plan(
        plan,
        army_registry,
        &frontline_registry.division_frontline_map,
    );
    let any_reachable = division_ids.iter().any(|division_id| {
        military_registry
            .divisions
            .get(division_id)
            .is_some_and(|division| {
                uncaptured.iter().any(|&target| {
                    find_path(
                        division.current_state,
                        target,
                        state_registry,
                        &[country_id],
                        &[enemy_id],
                    )
                    .is_some()
                })
            })
    });

    if any_reachable {
        OffensiveLineProgress::InProgress
    } else {
        OffensiveLineProgress::NoReachableTargets
    }
}

/// 停止命令処理 (Stopped Stance)
///
/// P21-006: 直接割当に加え、Army経由で防御配置の対象になりうるDivision
/// (`resolve_effective_division_ids_for_plan`と同じ解決規則)も対象に含める。
/// Defend/Offensive時に生成された移動を停止するだけであり、新しい移動やAttack命令を
/// 生成することはない(既存の停止処理の対象範囲を、Army経由分にも一貫させるだけ)。
pub fn process_stopped_plan(
    plan: &FrontlinePlan,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    army_registry: &ArmyRegistry,
) {
    let division_ids = resolve_effective_division_ids_for_plan(
        plan,
        army_registry,
        &frontline_registry.division_frontline_map,
    );
    for division_id in division_ids {
        if frontline_registry
            .frontline_generated_movements
            .contains(&division_id)
            && let Some(division) = military_registry.divisions.get_mut(&division_id)
            && division.status == DivisionStatus::Moving
        {
            division.status = DivisionStatus::Idle;
            division.destination = None;
            division.current_path.clear();
            division.target_state = None;
            division.movement_progress = 0.0;
        }
        frontline_registry
            .frontline_generated_movements
            .remove(&division_id);
    }
}

/// 日次作戦命令の実行
pub fn process_daily_frontline_plans(
    war_registry: &WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    frontline_registry: &mut FrontlineRegistry,
    current_date: Option<&str>,
    army_registry: &ArmyRegistry,
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
                    process_stopped_plan(
                        &plan,
                        military_registry,
                        frontline_registry,
                        army_registry,
                    );
                }
                FrontlineStance::Defend => {
                    cancel_stale_offensive_movements_before_defend(
                        &frontline,
                        &plan,
                        military_registry,
                        frontline_registry,
                        army_registry,
                    );
                    process_defensive_plan(
                        &frontline,
                        &plan,
                        state_registry,
                        military_registry,
                        war_registry,
                        frontline_registry,
                        army_registry,
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
                        army_registry,
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
    army_registry: Res<ArmyRegistry>,
) {
    for _ in day_events.read() {
        let current_date = date.display();
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            Some(&current_date),
            &army_registry,
        );
    }
}

/// P21-005: `ArmyRegistry`が変化した際(解散・自動解散・除外等、UI操作で日次を待たず
/// 即座に起こりうる)、無効化されたArmyの前線割当参照を毎フレーム整理する。
/// `map::division_selection::prune_selected_division`と同じ`is_changed()`ガード付き
/// パターン(War/Frontline終了時の清掃は`remove_frontline`経由で別途保証されるため、
/// ここではArmy側の変化だけを監視すればよい)。
pub fn sync_army_frontline_references(
    army_registry: Res<ArmyRegistry>,
    mut frontline_registry: ResMut<FrontlineRegistry>,
) {
    if !army_registry.is_changed() {
        return;
    }
    frontline_registry.sanitize_army_references(&army_registry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{CountryId, DivisionDefinitionId, DivisionId, StateId, WarId};
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{
        Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType,
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

        // アクティブな戦争を作成 (Country 1 vs Country 2, Goal: State 3)
        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
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
    fn test_division_assignment_and_validation() {
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
        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);

        // C3 (無関係な国) の陸軍
        let a2 = Division {
            id: DivisionId(1),
            owner: CountryId(3),
            manpower: 1000,
            ..military_registry
                .divisions
                .get(&division_id)
                .unwrap()
                .clone()
        };
        let other_division_id = military_registry.add_division(a2);

        // 割当成功の検証
        assert!(
            frontline_registry
                .assign_division(
                    division_id,
                    fl_id,
                    CountryId(1),
                    &military_registry,
                    &war_registry
                )
                .is_ok()
        );
        assert_eq!(
            frontline_registry.division_frontline_map.get(&division_id),
            Some(&fl_id)
        );

        // 重複登録の防止
        assert!(
            frontline_registry
                .assign_division(
                    division_id,
                    fl_id,
                    CountryId(1),
                    &military_registry,
                    &war_registry
                )
                .is_ok()
        );
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(
            plan.assigned_division_ids
                .iter()
                .filter(|&&id| id == division_id)
                .count(),
            1
        );

        // 他国陸軍の割り当て拒否
        assert!(
            frontline_registry
                .assign_division(
                    other_division_id,
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
                .unassign_division(division_id, CountryId(1), &military_registry)
                .is_ok()
        );
        assert!(
            !frontline_registry
                .division_frontline_map
                .contains_key(&division_id)
        );
    }

    /// P21-002回帰テスト: `unassign_division`は所有者と異なる`country_id`を渡すと拒否し、
    /// 割当状態を一切変更しない。UI経由(選択中陸軍は所有者不問で選択され得る)で
    /// 他国の前線割当を無断解除できてしまう不具合の修正確認。
    #[test]
    fn test_unassign_division_rejects_non_owner() {
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

        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);
        frontline_registry
            .assign_division(
                division_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();

        // CountryId(1)の陸軍を、無関係な第三国CountryId(3)として解除しようとすると拒否される
        let result =
            frontline_registry.unassign_division(division_id, CountryId(3), &military_registry);
        assert!(result.is_err());
        assert_eq!(
            frontline_registry.division_frontline_map.get(&division_id),
            Some(&fl_id),
            "unauthorized unassign must not change the assignment"
        );
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert!(plan.assigned_division_ids.contains(&division_id));

        // 存在しない陸軍IDも拒否される
        let result_missing =
            frontline_registry.unassign_division(DivisionId(999), CountryId(1), &military_registry);
        assert!(result_missing.is_err());
    }

    #[test]
    fn test_evaluate_frontline_division_command_feasibility() {
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
            evaluate_frontline_division_command_feasibility(
                &[],
                CountryId(1),
                &military_registry,
                None
            ),
            FrontlineCommandFeasibility::NoActiveFrontline
        );

        // 前線はあるが陸軍未選択
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NoDivisionSelected
        );

        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);

        // 自国の有効な陸軍を選択中 → Ready
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[division_id],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::Ready
        );

        // 他国の陸軍を選択中(所有者不問で選択され得るため) → NotOwnDivision
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[division_id],
                CountryId(2),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NotOwnDivision
        );

        // 存在しない陸軍IDを選択中 → DivisionNotFound
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[DivisionId(999)],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::DivisionNotFound
        );

        // P21-003: 複数選択のうち1件でも実行可能ならReady
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[DivisionId(999), division_id],
                CountryId(1),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::Ready
        );

        // P21-003: 全滅の場合は先頭(DivisionId昇順)の失敗理由を返す
        assert_eq!(
            evaluate_frontline_division_command_feasibility(
                &[division_id, DivisionId(999)],
                CountryId(2),
                &military_registry,
                Some(&frontline),
            ),
            FrontlineCommandFeasibility::NotOwnDivision
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
        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);
        let _ = frontline_registry.assign_division(
            division_id,
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
            &ArmyRegistry::default(),
        );

        // 割当部隊が自国側前線 State 2 へ向かって移動を開始したことを確認
        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(2)));
        assert_eq!(division.current_path, vec![StateId(2)]);
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
        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);
        let _ = frontline_registry.assign_division(
            division_id,
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
            &ArmyRegistry::default(),
        );

        // 待機中の部隊が隣接する戦争目標 State 3 へ進軍を開始したことを検証
        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(3)));
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

        let a1 = Division {
            id: DivisionId(0),
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
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(1),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_id = military_registry.add_division(a1);
        let _ = frontline_registry.assign_division(
            division_id,
            fl_id,
            CountryId(1),
            &military_registry,
            &war_registry,
        );

        // プレイヤーが手動で移動指示を出した想定
        if let Some(division) = military_registry.divisions.get_mut(&division_id) {
            division.status = DivisionStatus::Moving;
            division.destination = Some(StateId(2));
            division.current_path = vec![StateId(2)];
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
            &ArmyRegistry::default(),
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(2)));

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
            &ArmyRegistry::default(),
        );

        let division_after_stop = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division_after_stop.status, DivisionStatus::Moving);
        assert_eq!(division_after_stop.destination, Some(StateId(2)));
    }

    // ─── P21-005: Army↔Frontline割当 ────────────────────────────────────────

    use crate::military::army::ArmyRegistry;

    fn make_test_division(id: usize, owner: CountryId, state: StateId) -> Division {
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

    /// C1所有のArmy(所属師団1体)を1体作って返す。返り値は(ArmyId, DivisionId)。
    fn setup_army_for_c1(
        military_registry: &mut MilitaryRegistry,
        army_registry: &mut ArmyRegistry,
        state: StateId,
    ) -> (ArmyId, DivisionId) {
        let division = make_test_division(0, CountryId(1), state);
        let division_id = military_registry.add_division(division);
        let army_id = army_registry
            .create_army(CountryId(1), &[division_id], military_registry)
            .unwrap();
        (army_id, division_id)
    }

    #[test]
    fn test_assign_army_to_frontline_succeeds() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        assert!(
            frontline_registry
                .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                .is_ok()
        );
        assert_eq!(frontline_registry.frontline_for_army(army_id), Some(fl_id));
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(plan.assigned_army_ids, vec![army_id]);
    }

    #[test]
    fn test_multiple_armies_can_share_one_frontline() {
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

        let mut army_registry = ArmyRegistry::default();
        let d1 = military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
        let d2 = military_registry.add_division(make_test_division(1, CountryId(1), StateId(1)));
        let a1 = army_registry
            .create_army(CountryId(1), &[d1], &military_registry)
            .unwrap();
        let a2 = army_registry
            .create_army(CountryId(1), &[d2], &military_registry)
            .unwrap();

        frontline_registry
            .assign_army(a1, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .assign_army(a2, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(plan.assigned_army_ids, vec![a1, a2]);
    }

    #[test]
    fn test_army_belongs_to_at_most_one_frontline_reassignment_removes_old() {
        let (state_registry, mut war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id_1 = frontline_registry
            .get_frontline_for_war(WarId(0))
            .unwrap()
            .frontline_id;

        // 2つ目の戦争(C1 vs C2、別Frontline)を作る
        let war2 = War {
            id: WarId(1),
            name: "Second War".to_string(),
            attackers: [CountryId(1)].into_iter().collect(),
            defenders: [CountryId(2)].into_iter().collect(),
            primary_attacker: None,
            primary_defender: None,
            war_goals: vec![],
            start_date: "1800/02/01".to_string(),
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
        war_registry.wars.insert(war2.id, war2);
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );
        let fl_id_2 = frontline_registry
            .get_frontline_for_war(WarId(1))
            .unwrap()
            .frontline_id;
        assert_ne!(fl_id_1, fl_id_2);

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        frontline_registry
            .assign_army(
                army_id,
                fl_id_1,
                CountryId(1),
                &army_registry,
                &war_registry,
            )
            .unwrap();
        frontline_registry
            .assign_army(
                army_id,
                fl_id_2,
                CountryId(1),
                &army_registry,
                &war_registry,
            )
            .unwrap();

        assert_eq!(
            frontline_registry.frontline_for_army(army_id),
            Some(fl_id_2)
        );
        assert!(
            !frontline_registry
                .get_plan(fl_id_1, CountryId(1))
                .unwrap()
                .assigned_army_ids
                .contains(&army_id)
        );
        assert_eq!(
            frontline_registry
                .get_plan(fl_id_2, CountryId(1))
                .unwrap()
                .assigned_army_ids,
            vec![army_id]
        );
    }

    #[test]
    fn test_unassign_army_removes_assignment_and_is_idempotent() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        assert!(
            frontline_registry
                .unassign_army(army_id, CountryId(1), &army_registry)
                .is_ok()
        );
        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
        assert!(
            !frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .assigned_army_ids
                .contains(&army_id)
        );

        // 複数回解除しても安全(2回目もOk、状態も変化しない)
        assert!(
            frontline_registry
                .unassign_army(army_id, CountryId(1), &army_registry)
                .is_ok()
        );
        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
    }

    #[test]
    fn test_assign_army_rejects_nonexistent_army_id() {
        let (state_registry, war_registry, military_registry, mut frontline_registry) =
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
        let army_registry = ArmyRegistry::default();

        let result = frontline_registry.assign_army(
            ArmyId(999),
            fl_id,
            CountryId(1),
            &army_registry,
            &war_registry,
        );
        assert!(result.is_err());
        assert_eq!(frontline_registry.frontline_for_army(ArmyId(999)), None);
    }

    #[test]
    fn test_assign_army_rejects_nonexistent_frontline_id() {
        let (_, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        let result = frontline_registry.assign_army(
            army_id,
            FrontlineId(999),
            CountryId(1),
            &army_registry,
            &war_registry,
        );
        assert!(result.is_err());
        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
    }

    #[test]
    fn test_assign_army_rejects_foreign_owned_army() {
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

        // C2(このFrontlineの参加国ではあるが、C1として割り当てようとする)所有のArmy
        let mut army_registry = ArmyRegistry::default();
        let division_id =
            military_registry.add_division(make_test_division(0, CountryId(2), StateId(3)));
        let army_id = army_registry
            .create_army(CountryId(2), &[division_id], &military_registry)
            .unwrap();

        // C1として割り当てようとすると拒否される(Army所有者はC2)
        let result = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(1),
            &army_registry,
            &war_registry,
        );
        assert!(result.is_err());
        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
    }

    #[test]
    fn test_assign_army_rejects_non_participant_country() {
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

        // C3は戦争(C1 vs C2)の参加国ではない
        let mut army_registry = ArmyRegistry::default();
        let division_id =
            military_registry.add_division(make_test_division(0, CountryId(3), StateId(1)));
        let army_id = army_registry
            .create_army(CountryId(3), &[division_id], &military_registry)
            .unwrap();

        let result = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(3),
            &army_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_army_rejects_ended_war_frontline() {
        let (state_registry, mut war_registry, mut military_registry, mut frontline_registry) =
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

        // 戦争を終了させる(Frontlineレコード自体はまだ残っている状態を模擬)
        war_registry.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::WhitePeace;

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        let result = frontline_registry.assign_army(
            army_id,
            fl_id,
            CountryId(1),
            &army_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_army_ordering_is_deterministic_not_hashmap_order() {
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

        let mut army_registry = ArmyRegistry::default();
        let d1 = military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
        let d2 = military_registry.add_division(make_test_division(1, CountryId(1), StateId(1)));
        let d3 = military_registry.add_division(make_test_division(2, CountryId(1), StateId(1)));
        let a1 = army_registry
            .create_army(CountryId(1), &[d1], &military_registry)
            .unwrap();
        let a2 = army_registry
            .create_army(CountryId(1), &[d2], &military_registry)
            .unwrap();
        let a3 = army_registry
            .create_army(CountryId(1), &[d3], &military_registry)
            .unwrap();

        // わざと降順で割り当てる
        for &army_id in &[a3, a1, a2] {
            frontline_registry
                .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                .unwrap();
        }

        let mut expected = [a1, a2, a3];
        expected.sort_by_key(|id| id.0);
        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert_eq!(
            plan.assigned_army_ids, expected,
            "assigned_army_ids must be ArmyId-sorted regardless of insertion order"
        );
    }

    #[test]
    fn test_army_member_division_changes_do_not_affect_frontline_assignment() {
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

        let mut army_registry = ArmyRegistry::default();
        let d1 = military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
        let d2 = military_registry.add_division(make_test_division(1, CountryId(1), StateId(1)));
        let army_id = army_registry
            .create_army(CountryId(1), &[d1], &military_registry)
            .unwrap();

        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        // Armyへ師団を追加してもFrontline割当は維持される
        army_registry
            .add_division(army_id, d2, CountryId(1), &military_registry)
            .unwrap();
        frontline_registry.sanitize_army_references(&army_registry);
        assert_eq!(frontline_registry.frontline_for_army(army_id), Some(fl_id));

        // Armyから師団を除外してもFrontline割当は維持される(Army自体は残る)
        army_registry
            .remove_division(d1, CountryId(1), &military_registry)
            .unwrap();
        frontline_registry.sanitize_army_references(&army_registry);
        assert_eq!(frontline_registry.frontline_for_army(army_id), Some(fl_id));
    }

    #[test]
    fn test_army_disband_cleans_up_frontline_assignment() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        army_registry.disband(army_id, CountryId(1)).unwrap();
        frontline_registry.sanitize_army_references(&army_registry);

        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
        assert!(
            !frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .assigned_army_ids
                .contains(&army_id)
        );
    }

    #[test]
    fn test_frontline_removal_cleans_up_army_assignment() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        frontline_registry.remove_frontline(fl_id, &military_registry);

        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
        assert!(!frontline_registry.army_frontline_map.contains_key(&army_id));
    }

    #[test]
    fn test_war_end_cleans_up_army_assignment() {
        let (state_registry, mut war_registry, mut military_registry, mut frontline_registry) =
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        war_registry.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::WhitePeace;
        // sanitize_references(Division版)は無効化された戦争のFrontlineをremove_frontline
        // 経由で削除する。この既存経路がArmy割当も一緒に清掃することを確認する。
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        assert_eq!(frontline_registry.frontline_for_army(army_id), None);
        assert!(frontline_registry.frontlines.is_empty());
    }

    #[test]
    fn test_army_assignment_does_not_mutate_any_division_fields() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        let before = military_registry
            .divisions
            .get(&division_id)
            .unwrap()
            .clone();

        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        let after = military_registry
            .divisions
            .get(&division_id)
            .unwrap()
            .clone();
        assert_eq!(before.current_state, after.current_state);
        assert_eq!(before.destination, after.destination);
        assert_eq!(before.target_state, after.target_state);
        assert_eq!(before.current_path, after.current_path);
        assert_eq!(before.movement_progress, after.movement_progress);
        assert_eq!(before.status, after.status);
        assert_eq!(before.combat_id, after.combat_id);
        assert!(
            !frontline_registry
                .frontline_generated_movements
                .contains(&division_id),
            "Army assignment alone must never register frontline_generated_movements"
        );

        // 解除でも変化しない
        frontline_registry
            .unassign_army(army_id, CountryId(1), &army_registry)
            .unwrap();
        let after_unassign = military_registry
            .divisions
            .get(&division_id)
            .unwrap()
            .clone();
        assert_eq!(before.current_state, after_unassign.current_state);
        assert_eq!(before.status, after_unassign.status);
        assert_eq!(before.destination, after_unassign.destination);
    }

    /// P21-005時点では「Army割当だけでは(スタンスを設定しても)自動移動は一切発生しない」
    /// ことを検証していたが、P21-006で「Armyの前線割当を既存の防御配置処理へ接続する」
    /// ことが要件化されたため、このテスト自体がP21-006の意図的な仕様変更の対象になった。
    /// 削除はせず、検証内容をP21-006時点の正しい仕様(Army経由Divisionも
    /// process_defensive_plan経由で防御配置される)へ更新する。P21-005当時の
    /// 「割当操作自体は何もしない」という不変条件は
    /// `test_army_assignment_alone_without_stance_change_does_not_move_division`
    /// (このすぐ下)へ引き継ぎ、狭められた形で維持する。
    #[test]
    fn test_army_assignment_with_defend_stance_generates_defensive_placement_p21_006() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        // P21-006: Defendスタンスを設定すると、Army経由(直接割当ではない)のDivisionも
        // 既存のprocess_defensive_plan経由で自国側前線(State 2)へ移動指示を受ける
        // (直接割当済みDivisionと全く同じ経路・同じ配置ロジック)。
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(2)));
        assert_eq!(division.current_path, vec![StateId(2)]);
        assert!(
            frontline_registry
                .frontline_generated_movements
                .contains(&division_id),
            "Army経由の防御配置もfrontline_generated_movementsへ記録され、Stopped/手動移動の\
             優先度判定が既存Divisionと同じ経路で機能するはず"
        );
    }

    /// P21-005由来の不変条件を、P21-006後も真であり続ける狭い形で維持する:
    /// `assign_army`自体(Planのstanceを明示的に変更しない限り、新規Planは既定で
    /// `FrontlineStance::Stopped`)は、それだけでは一切のDivision移動を発生させない。
    #[test]
    fn test_army_assignment_alone_without_stance_change_does_not_move_division() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        // スタンスは変更しない(新規Planは既定でStopped)。
        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .stance,
            FrontlineStance::Stopped
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert_eq!(division.destination, None);
        assert!(
            !frontline_registry
                .frontline_generated_movements
                .contains(&division_id)
        );
    }

    #[test]
    fn test_evaluate_army_frontline_assign_feasibility() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry) =
            setup_test_environment();
        update_all_frontlines(
            &war_registry,
            &state_registry,
            &military_registry,
            &mut frontline_registry,
        );

        let mut army_registry = ArmyRegistry::default();

        // 未選択
        assert_eq!(
            evaluate_army_frontline_assign_feasibility(
                None,
                CountryId(1),
                &army_registry,
                &frontline_registry,
                &war_registry,
            ),
            ArmyFrontlineAssignFeasibility::NoArmySelected
        );

        // 存在しないArmyId
        assert_eq!(
            evaluate_army_frontline_assign_feasibility(
                Some(ArmyId(999)),
                CountryId(1),
                &army_registry,
                &frontline_registry,
                &war_registry,
            ),
            ArmyFrontlineAssignFeasibility::ArmyNotFound
        );

        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));

        // 他国として評価すると所有者不一致
        assert_eq!(
            evaluate_army_frontline_assign_feasibility(
                Some(army_id),
                CountryId(2),
                &army_registry,
                &frontline_registry,
                &war_registry,
            ),
            ArmyFrontlineAssignFeasibility::NotOwnArmy
        );

        // 自国・有効な前線あり → Ready
        assert_eq!(
            evaluate_army_frontline_assign_feasibility(
                Some(army_id),
                CountryId(1),
                &army_registry,
                &frontline_registry,
                &war_registry,
            ),
            ArmyFrontlineAssignFeasibility::Ready
        );

        // 割当可能な前線一覧がArmyId/FrontlineId順に決定的であることも併せて確認
        let assignable =
            frontline_registry.assignable_frontlines_for_army(CountryId(1), &war_registry);
        assert_eq!(assignable.len(), 1);
    }

    // ─── P21-006: Army前線割当 → 既存防御配置処理への接続 ─────────────────────

    /// 要求テスト項目1: Army割当済み・直接割当なしの所属Divisionが防御配置対象になる。
    #[test]
    fn test_resolve_includes_army_member_without_direct_assignment() {
        let owner = CountryId(1);
        let fl_id = FrontlineId(0);
        let mut plan = FrontlinePlan::new(fl_id, owner);

        let d1 = DivisionId(1);
        let mut military_registry = MilitaryRegistry::default();
        military_registry
            .divisions
            .insert(d1, make_test_division(1, owner, StateId(1)));

        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(owner, &[d1], &military_registry)
            .unwrap();
        plan.assigned_army_ids = vec![army_id];

        let division_frontline_map: HashMap<DivisionId, FrontlineId> = HashMap::new();
        let resolved =
            resolve_effective_division_ids_for_plan(&plan, &army_registry, &division_frontline_map);
        assert_eq!(resolved, vec![d1]);

        assert_eq!(
            resolve_effective_frontline_for_division(
                d1,
                &FrontlineRegistry::default(),
                &army_registry
            ),
            None,
            "division_frontline_mapが空のFrontlineRegistryでは、army_frontline_mapも\
             空なのでNone(このassertはヘルパーの単純な事前条件確認)"
        );
    }

    /// 要求テスト項目2: Army未割当のPlanは既存挙動のまま(直接割当のみが対象)。
    #[test]
    fn test_resolve_without_army_assignment_matches_existing_behavior() {
        let owner = CountryId(1);
        let fl_id = FrontlineId(0);
        let mut plan = FrontlinePlan::new(fl_id, owner);
        plan.assigned_division_ids = vec![DivisionId(5), DivisionId(2)];

        let army_registry = ArmyRegistry::default();
        let division_frontline_map: HashMap<DivisionId, FrontlineId> = HashMap::new();
        let resolved =
            resolve_effective_division_ids_for_plan(&plan, &army_registry, &division_frontline_map);
        assert_eq!(
            resolved,
            vec![DivisionId(2), DivisionId(5)],
            "Army未割当ならassigned_division_idsそのまま(ソート済み)が返るはず"
        );
    }

    /// 要求テスト項目3: Divisionの直接割当がArmy継承より優先される。直接割当を持つ
    /// Divisionは、それが所属するArmyの割当先が同じPlanであっても、Army経由の
    /// 追加処理で二重に扱われない(直接割当が唯一の経路として使われる)。
    #[test]
    fn test_resolve_direct_assignment_takes_priority_over_army_inheritance() {
        let owner = CountryId(1);
        let fl_id = FrontlineId(0);
        let mut plan = FrontlinePlan::new(fl_id, owner);

        let d1 = DivisionId(1); // 直接割当あり
        let d2 = DivisionId(2); // 直接割当なし、Army経由のみ

        plan.assigned_division_ids = vec![d1];

        let mut military_registry = MilitaryRegistry::default();
        military_registry
            .divisions
            .insert(d1, make_test_division(1, owner, StateId(1)));
        military_registry
            .divisions
            .insert(d2, make_test_division(2, owner, StateId(1)));

        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(owner, &[d1, d2], &military_registry)
            .unwrap();
        plan.assigned_army_ids = vec![army_id];

        let mut division_frontline_map: HashMap<DivisionId, FrontlineId> = HashMap::new();
        division_frontline_map.insert(d1, fl_id);

        let resolved =
            resolve_effective_division_ids_for_plan(&plan, &army_registry, &division_frontline_map);
        assert_eq!(
            resolved,
            vec![d1, d2],
            "d1は直接割当経由で1回だけ、d2はArmy経由で含まれるはず"
        );
    }

    /// 要求テスト項目4: 直接割当とArmy割当が同一Frontlineでも、実際の日次処理
    /// (process_daily_frontline_plans)で二重に移動指示が生成されたり状態が壊れたり
    /// しない。
    #[test]
    fn test_direct_and_army_assignment_to_same_frontline_is_not_double_processed() {
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

        let division_id =
            military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(CountryId(1), &[division_id], &military_registry)
            .unwrap();

        // 同じDivisionを直接割当と、そのArmy経由の割当の両方で対象にする。
        frontline_registry
            .assign_division(
                division_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        // 解決結果にdivision_idが1回だけ含まれることを確認(直接割当優先による重複排除)。
        let plan = frontline_registry
            .get_plan(fl_id, CountryId(1))
            .unwrap()
            .clone();
        let resolved = resolve_effective_division_ids_for_plan(
            &plan,
            &army_registry,
            &frontline_registry.division_frontline_map,
        );
        assert_eq!(resolved, vec![division_id]);

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        // 単一の一貫した移動指示になっていること(壊れた/矛盾した状態でないこと)を確認。
        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(2)));
        assert_eq!(division.current_path, vec![StateId(2)]);
    }

    /// 要求テスト項目5: ArmyへDivisionを追加すると、次回の日次処理から対象になる。
    #[test]
    fn test_adding_division_to_assigned_army_includes_it_from_next_processing() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, d1) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        // 1回目の処理(d1のみ)
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );
        let d1_after_first = military_registry.divisions.get(&d1).unwrap().clone();
        assert_eq!(d1_after_first.status, DivisionStatus::Moving);

        // d2を新規追加してArmyへ加入させる
        let d2 = military_registry.add_division(make_test_division(9, CountryId(1), StateId(1)));
        army_registry
            .add_division(army_id, d2, CountryId(1), &military_registry)
            .unwrap();
        assert!(d2 != d1_after_first.id);

        // 2回目の処理: d2も対象になるはず
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );
        let d2_after = military_registry.divisions.get(&d2).unwrap();
        assert_eq!(
            d2_after.status,
            DivisionStatus::Moving,
            "Armyへ追加した師団は次回処理から防御配置の対象になるはず"
        );
        assert_eq!(d2_after.destination, Some(StateId(2)));
    }

    /// 要求テスト項目6: ArmyからDivisionを外すと、次回の日次処理から対象外になる。
    #[test]
    fn test_removing_division_from_assigned_army_excludes_it_from_next_processing() {
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

        let d1 = military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
        let d2 = military_registry.add_division(make_test_division(1, CountryId(1), StateId(1)));
        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(CountryId(1), &[d1, d2], &military_registry)
            .unwrap();
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        // d2をArmyから除外
        army_registry
            .remove_division(d2, CountryId(1), &military_registry)
            .unwrap();

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let d1_after = military_registry.divisions.get(&d1).unwrap();
        assert_eq!(
            d1_after.status,
            DivisionStatus::Moving,
            "Armyに残っているd1は引き続き対象になるはず"
        );

        let d2_after = military_registry.divisions.get(&d2).unwrap();
        assert_eq!(
            d2_after.status,
            DivisionStatus::Idle,
            "Armyから外れたd2は防御配置の対象外になるはず"
        );
        assert_eq!(d2_after.destination, None);
    }

    /// 要求テスト項目7: Army前線解除後は、そのArmy経由の新しい配置要求が生成されない。
    #[test]
    fn test_unassigning_army_frontline_stops_generating_new_placement() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        // 割当解除
        frontline_registry
            .unassign_army(army_id, CountryId(1), &army_registry)
            .unwrap();

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "前線割当を解除した後は、そのArmy経由の新しい配置要求が生成されないはず"
        );
        assert_eq!(division.destination, None);
        assert!(
            !frontline_registry
                .frontline_generated_movements
                .contains(&division_id)
        );
    }

    /// 要求テスト項目8: 複数Armyを同一Frontlineへ割り当てても、挿入順に関わらず
    /// 決定的に処理される(HashMapの反復順に依存しない)。
    #[test]
    fn test_multiple_armies_on_same_frontline_process_deterministically() {
        fn run_scenario(insert_a1_first: bool) -> (DivisionStatus, DivisionStatus) {
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

            let d1 =
                military_registry.add_division(make_test_division(0, CountryId(1), StateId(1)));
            let d2 =
                military_registry.add_division(make_test_division(1, CountryId(1), StateId(1)));
            let mut army_registry = ArmyRegistry::default();
            let a1 = army_registry
                .create_army(CountryId(1), &[d1], &military_registry)
                .unwrap();
            let a2 = army_registry
                .create_army(CountryId(1), &[d2], &military_registry)
                .unwrap();

            let assign_order: [ArmyId; 2] = if insert_a1_first { [a1, a2] } else { [a2, a1] };
            for army_id in assign_order {
                frontline_registry
                    .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
                    .unwrap();
            }
            if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
                plan.stance = FrontlineStance::Defend;
            }

            process_daily_frontline_plans(
                &war_registry,
                &state_registry,
                &mut military_registry,
                &mut frontline_registry,
                None,
                &army_registry,
            );

            (
                military_registry.divisions.get(&d1).unwrap().status,
                military_registry.divisions.get(&d2).unwrap().status,
            )
        }

        let result_a = run_scenario(true);
        let result_b = run_scenario(false);
        assert_eq!(
            result_a, result_b,
            "Army割当の挿入順を変えても最終結果は決定的に一致するはず"
        );
        assert_eq!(result_a, (DivisionStatus::Moving, DivisionStatus::Moving));
    }

    /// 要求テスト項目9: owner/country不一致の不正状態を通さない(通常API経由では
    /// 起こらないが、防御的にresolve関数自体が除外することを直接確認する)。
    #[test]
    fn test_resolve_excludes_army_with_owner_country_mismatch() {
        let owner = CountryId(1);
        let fl_id = FrontlineId(0);
        let mut plan = FrontlinePlan::new(fl_id, owner);

        let d1 = DivisionId(1);
        let mut military_registry = MilitaryRegistry::default();
        military_registry
            .divisions
            .insert(d1, make_test_division(1, CountryId(2), StateId(1)));

        // C2所有のArmyを、C1のPlanへ(通常API外の手段で)不正に紐付けた状態を模擬する。
        let mut army_registry = ArmyRegistry::default();
        let army_id = army_registry
            .create_army(CountryId(2), &[d1], &military_registry)
            .unwrap();
        plan.assigned_army_ids = vec![army_id];

        let division_frontline_map: HashMap<DivisionId, FrontlineId> = HashMap::new();
        let resolved =
            resolve_effective_division_ids_for_plan(&plan, &army_registry, &division_frontline_map);
        assert!(
            resolved.is_empty(),
            "owner不一致のArmyはPlanのcommanding_country_idと矛盾するため除外されるはず"
        );
    }

    /// 要求テスト項目10: セーブ/ロード相当(RON往復)後もArmy経由の有効配置が再現される。
    #[test]
    fn test_army_driven_placement_survives_ron_round_trip() {
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        // RON往復(SaveGameV1全体ではなく、対象データだけを直接シリアライズする軽量な
        // 等価テスト。P21-005のarmy_frontline_map/assigned_army_idsのシリアライズ経路
        // 自体は`save::dto`側で既に検証済み。MilitaryRegistry自体はSerialize/Deserialize
        // を実装しないため、内部のdivisions HashMapだけを往復させ、`from_saved_parts`で
        // 再構築する)。
        let frontline_ron = ron::to_string(&frontline_registry).unwrap();
        let army_ron = ron::to_string(&army_registry).unwrap();
        let divisions_ron = ron::to_string(&military_registry.divisions).unwrap();

        let mut restored_frontline: FrontlineRegistry = ron::from_str(&frontline_ron).unwrap();
        let restored_army: ArmyRegistry = ron::from_str(&army_ron).unwrap();
        let restored_divisions: HashMap<DivisionId, Division> =
            ron::from_str(&divisions_ron).unwrap();
        let mut restored_military = MilitaryRegistry::from_saved_parts(
            HashMap::new(),
            restored_divisions,
            military_registry.next_division_id(),
        );

        assert_eq!(
            restored_frontline.frontline_for_army(army_id),
            Some(fl_id),
            "RON往復後もArmy前線割当が保持されるはず"
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut restored_military,
            &mut restored_frontline,
            None,
            &restored_army,
        );

        let division = restored_military.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Moving,
            "RON往復後もArmy経由の防御配置が再現されるはず"
        );
        assert_eq!(division.destination, Some(StateId(2)));
    }

    /// 要求テスト項目11: 旧形式(assigned_army_ids/army_frontline_mapフィールドが
    /// 存在しないRON)から復元したPlan/Registryでは、Army由来の配置が発生しない
    /// (P21-005の`#[serde(default)]`互換性が、P21-006の新しい処理経路でも保たれる)。
    #[test]
    fn test_old_format_restored_plan_generates_no_army_driven_placement() {
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

        // 直接割当は使わず、Army割当だけを行う。
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        // Planを旧形式相当(assigned_army_idsフィールド欠落)のRONへ変換してから復元する。
        let plan = frontline_registry
            .get_plan(fl_id, CountryId(1))
            .unwrap()
            .clone();
        let plan_ron = ron::to_string(&plan).unwrap();
        assert!(plan_ron.contains("assigned_army_ids"));
        let old_format_ron =
            plan_ron.replacen(&format!(",assigned_army_ids:[({})]", army_id.0), "", 1);
        assert_ne!(
            old_format_ron, plan_ron,
            "テスト前提: フィールドを実際に除去できていること"
        );
        let restored_plan: FrontlinePlan = ron::from_str(&old_format_ron).unwrap();
        assert!(restored_plan.assigned_army_ids.is_empty());

        // 旧形式相当のPlanを差し替えてから日次処理を実行する。
        frontline_registry
            .plans
            .insert((fl_id, CountryId(1)), restored_plan);
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "assigned_army_idsを欠いた(旧形式相当の)PlanではArmy由来の配置は発生しないはず"
        );
    }

    /// 要求テスト項目12: 前線削除後は、army_frontline_map等の参照が残らず、
    /// 日次処理を実行してもパニックや新規配置が発生しない。
    #[test]
    fn test_process_daily_frontline_plans_is_safe_after_frontline_removed() {
        let (state_registry, mut war_registry, mut military_registry, mut frontline_registry) =
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

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        frontline_registry.remove_frontline(fl_id, &military_registry);
        assert!(!frontline_registry.army_frontline_map.contains_key(&army_id));

        // Warも終結させておく(そうしないとprocess_daily_frontline_plans内部の
        // update_all_frontlinesが、まだActiveな戦争に対して前線を再生成してしまい、
        // 「前線削除後」の状態を維持できない)。
        war_registry.wars.get_mut(&WarId(0)).unwrap().status = WarStatus::WhitePeace;

        // パニックしないこと、かつ新規配置が発生しないことを確認する。
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert!(frontline_registry.frontlines.is_empty());
    }

    // ─── P21-007: 攻勢線(計画データのみ) ──────────────────────────────────────

    /// 攻勢線テスト専用の世界。`setup_test_environment`より広い州構成を持つ:
    /// State1,2 = C1(攻撃側)所有。State2は前線国境地域。
    /// State3,4 = C2(防御側)所有。State3は前線国境地域、State4はState3と隣接する
    ///            敵内陸地域(3-4で連結集合になる)。
    /// State5   = C2所有だがState3/4のいずれとも隣接しない孤立した敵地域
    ///            (非連結判定のテスト用)。
    /// State6   = C3(この戦争の非参加国)所有の陸地(第三国領域の拒否テスト用)。
    /// State7   = 海域(海域拒否テスト用)。
    fn setup_offensive_line_environment() -> (
        StateRegistry,
        WarRegistry,
        MilitaryRegistry,
        FrontlineRegistry,
        FrontlineId,
    ) {
        let (state_registry, war_registry, military_registry, mut frontline_registry) = {
            let s1 = StateData {
                id: StateId(1),
                owner_country_id: CountryId(1),
                neighbors: vec![StateId(2)],
                ..Default::default()
            };
            let s2 = StateData {
                id: StateId(2),
                owner_country_id: CountryId(1),
                neighbors: vec![StateId(1), StateId(3)],
                ..Default::default()
            };
            let s3 = StateData {
                id: StateId(3),
                owner_country_id: CountryId(2),
                neighbors: vec![StateId(2), StateId(4)],
                ..Default::default()
            };
            let s4 = StateData {
                id: StateId(4),
                owner_country_id: CountryId(2),
                neighbors: vec![StateId(3)],
                ..Default::default()
            };
            let s5 = StateData {
                id: StateId(5),
                owner_country_id: CountryId(2),
                neighbors: vec![],
                ..Default::default()
            };
            let s6 = StateData {
                id: StateId(6),
                owner_country_id: CountryId(3),
                neighbors: vec![],
                ..Default::default()
            };
            let s7 = StateData {
                id: StateId(7),
                owner_country_id: CountryId(0),
                neighbors: vec![],
                is_sea: true,
                ..Default::default()
            };
            let state_registry = StateRegistry::build(vec![s1, s2, s3, s4, s5, s6, s7]);

            let mut war_registry = WarRegistry::default();
            let war = War {
                id: WarId(0),
                name: "Offensive Line Test War".to_string(),
                attackers: [CountryId(1)].into_iter().collect(),
                defenders: [CountryId(2)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
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
                MilitaryRegistry::default(),
                FrontlineRegistry::default(),
            )
        };

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

        (
            state_registry,
            war_registry,
            military_registry,
            frontline_registry,
            fl_id,
        )
    }

    /// 要求テスト項目1: 有効な攻勢線を設定でき、取得できる。
    #[test]
    fn set_offensive_line_valid_single_region_can_be_read_back() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            Some(vec![StateId(4)])
        );
    }

    /// 要求テスト項目2: 攻勢線の再設定は既存値を完全に置き換える。
    #[test]
    fn set_offensive_line_overwrites_previous_value() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(5)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            Some(vec![StateId(5)])
        );
    }

    /// 要求テスト項目3: 攻勢線の解除。
    #[test]
    fn clear_offensive_line_removes_value() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        frontline_registry.clear_offensive_line(fl_id, CountryId(1));

        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            None
        );
    }

    /// 要求テスト項目2追加確認: 連結した複数地点の攻勢線も設定できる(StateId昇順で保持)。
    #[test]
    fn set_offensive_line_accepts_connected_multi_region() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4), StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            Some(vec![StateId(3), StateId(4)])
        );
    }

    /// 要求テスト項目4: 重複IDは拒否される。
    #[test]
    fn set_offensive_line_rejects_duplicate_region() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(4), StateId(4)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目5: 存在しないIDは拒否される。
    #[test]
    fn set_offensive_line_rejects_unknown_region() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(999)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目6a: 海域は拒否される。
    #[test]
    fn set_offensive_line_rejects_sea_region() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(7)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目6b: Frontline所有国の自国領は拒否される。
    #[test]
    fn set_offensive_line_rejects_own_territory() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(2)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目6c: この戦争の非参加国(第三国)の領域は拒否される。
    #[test]
    fn set_offensive_line_rejects_third_party_territory() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(6)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目7: 非連結な複数地点は拒否される(State4とState5は隣接しない)。
    #[test]
    fn set_offensive_line_rejects_disconnected_regions() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(4), StateId(5)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
    }

    /// 要求テスト項目8: 不正な設定要求では既存の攻勢線を変更しない。
    #[test]
    fn set_offensive_line_invalid_request_does_not_mutate_existing_value() {
        let (state_registry, war_registry, _, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        let result = frontline_registry.set_offensive_line(
            fl_id,
            CountryId(1),
            &[StateId(4), StateId(5)],
            &state_registry,
            &war_registry,
        );
        assert!(result.is_err());
        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids,
            Some(vec![StateId(4)]),
            "不正な要求の後も直前に設定した攻勢線が維持されているはず"
        );
    }

    /// 要求テスト項目9: 同じFrontlineを共有する複数Armyは同じ攻勢線を参照する
    /// (攻勢線はArmyごとではなくPlanに1つだけ保持されるため)。
    #[test]
    fn multiple_armies_sharing_frontline_plan_share_offensive_line() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let mut army_registry = ArmyRegistry::default();
        let (army1_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        let division2 = make_test_division(1, CountryId(1), StateId(1));
        let division2_id = military_registry.add_division(division2);
        let army2_id = army_registry
            .create_army(CountryId(1), &[division2_id], &military_registry)
            .unwrap();

        frontline_registry
            .assign_army(army1_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .assign_army(army2_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        let fl_via_army1 = frontline_registry.frontline_for_army(army1_id).unwrap();
        let fl_via_army2 = frontline_registry.frontline_for_army(army2_id).unwrap();
        assert_eq!(fl_via_army1, fl_via_army2);

        let line_via_army1 = frontline_registry
            .get_plan(fl_via_army1, CountryId(1))
            .unwrap()
            .offensive_line_region_ids
            .clone();
        let line_via_army2 = frontline_registry
            .get_plan(fl_via_army2, CountryId(1))
            .unwrap()
            .offensive_line_region_ids
            .clone();
        assert_eq!(line_via_army1, Some(vec![StateId(4)]));
        assert_eq!(line_via_army1, line_via_army2);
    }

    /// P21-008でP21-007時点の主張を意図的に反転させたテスト。
    ///
    /// P21-007時点では「攻勢線を設定しただけでは日次処理の移動・攻撃結果は一切変化しない」
    /// (`process_offensive_plan`から一切参照されない非実行保証)ことを検証していたが、
    /// P21-008の目的そのものが「攻勢線を実際の攻撃目標として接続する」ことであるため、
    /// この主張は仕様として意図的に反転する。ここでは同一の直接割当Division・同一の
    /// Offensive姿勢という条件のまま、攻勢線の有無だけで実際に結果が変わる
    /// (旧objective_region_idベースの隣接1ホップ攻撃[State3]ではなく、攻勢線の目標
    /// [State4、複数ホップ]へ向かう)ことを確認する。「攻勢線なしなら影響しない」という
    /// 旧来の主張のうち、Army経由Divisionに関する部分は
    /// `offensive_stance_without_line_does_not_attack_army_division`が、直接割当
    /// Divisionの旧来動作そのものは無改修の`test_offensive_operations_objective_and_attack`
    /// が、それぞれ引き続き検証する。
    #[test]
    fn setting_offensive_line_now_changes_daily_processing_outcome_by_design() {
        // offensive_line_region_idsなしのベースライン
        let (
            state_registry,
            war_registry,
            mut military_registry_a,
            mut frontline_registry_a,
            fl_id_a,
        ) = setup_offensive_line_environment();
        let (_, division_id_a) = setup_army_for_c1(
            &mut military_registry_a,
            &mut ArmyRegistry::default(),
            StateId(2),
        );
        frontline_registry_a
            .get_plan_mut(fl_id_a, CountryId(1))
            .unwrap()
            .assigned_division_ids = vec![division_id_a];
        if let Some(plan) = frontline_registry_a.get_plan_mut(fl_id_a, CountryId(1)) {
            plan.stance = FrontlineStance::Offensive;
        }
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry_a,
            &mut frontline_registry_a,
            None,
            &ArmyRegistry::default(),
        );
        let division_a = military_registry_a.divisions.get(&division_id_a).unwrap();
        let outcome_a = (
            division_a.status,
            division_a.destination,
            division_a.current_path.clone(),
        );

        // offensive_line_region_idsを設定した場合(同一の他条件)
        let (
            state_registry,
            war_registry,
            mut military_registry_b,
            mut frontline_registry_b,
            fl_id_b,
        ) = setup_offensive_line_environment();
        let (_, division_id_b) = setup_army_for_c1(
            &mut military_registry_b,
            &mut ArmyRegistry::default(),
            StateId(2),
        );
        frontline_registry_b
            .get_plan_mut(fl_id_b, CountryId(1))
            .unwrap()
            .assigned_division_ids = vec![division_id_b];
        frontline_registry_b
            .set_offensive_line(
                fl_id_b,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        if let Some(plan) = frontline_registry_b.get_plan_mut(fl_id_b, CountryId(1)) {
            plan.stance = FrontlineStance::Offensive;
        }
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry_b,
            &mut frontline_registry_b,
            None,
            &ArmyRegistry::default(),
        );
        let division_b = military_registry_b.divisions.get(&division_id_b).unwrap();
        let outcome_b = (
            division_b.status,
            division_b.destination,
            division_b.current_path.clone(),
        );

        assert_eq!(
            outcome_a,
            (DivisionStatus::Moving, Some(StateId(3)), vec![StateId(3)]),
            "攻勢線なしの場合、旧来のobjective_region_idベース隣接1ホップ攻撃のままのはず"
        );
        assert_eq!(
            outcome_b,
            (
                DivisionStatus::Moving,
                Some(StateId(4)),
                vec![StateId(3), StateId(4)]
            ),
            "攻勢線ありの場合、攻勢線の目標(State4)への複数ホップ経路を発行するはず"
        );
        assert_ne!(
            outcome_a, outcome_b,
            "P21-008により、攻勢線の有無で日次処理の結果は意図的に変化するはず(P21-007時点の非実行保証はここで終わる)"
        );
        assert!(
            !frontline_registry_a
                .frontline_generated_movements
                .is_empty()
                && !frontline_registry_b
                    .frontline_generated_movements
                    .is_empty(),
            "いずれのシナリオでもfrontline_generated_movementsへ記録されるはず(取消・置換規則の対象になる)"
        );
    }

    /// 要求テスト項目18: 攻勢線を設定していても、P21-006のArmy経由の防御配置は
    /// これまで通り機能する。
    #[test]
    fn army_driven_defensive_placement_still_works_with_offensive_line_set() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, CountryId(1)) {
            plan.stance = FrontlineStance::Defend;
        }

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Moving,
            "攻勢線が設定されていてもP21-006のArmy経由防御配置は維持されるはず"
        );
        assert_eq!(division.destination, Some(StateId(2)));
    }

    /// 要求テスト項目16: 前線削除(和平相当)は攻勢線もPlanごと完全に削除する。
    #[test]
    fn remove_frontline_drops_offensive_line_with_the_rest_of_the_plan() {
        let (state_registry, war_registry, military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        assert!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .offensive_line_region_ids
                .is_some()
        );

        frontline_registry.remove_frontline(fl_id, &military_registry);

        assert!(frontline_registry.get_plan(fl_id, CountryId(1)).is_none());
        assert!(frontline_registry.plans.is_empty());
    }

    /// Army解除だけではFrontlinePlan自体(と攻勢線)が残る。
    #[test]
    fn unassign_army_alone_leaves_offensive_line_intact() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();

        let mut army_registry = ArmyRegistry::default();
        let (army_id, _) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(1));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        frontline_registry
            .unassign_army(army_id, CountryId(1), &army_registry)
            .unwrap();

        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        assert!(plan.assigned_army_ids.is_empty());
        assert_eq!(plan.offensive_line_region_ids, Some(vec![StateId(4)]));
    }

    // ─── P21-008: 攻勢線を既存の攻撃処理へ接続 ──────────────────────────────────

    fn set_stance(
        frontline_registry: &mut FrontlineRegistry,
        fl_id: FrontlineId,
        country_id: CountryId,
        stance: FrontlineStance,
    ) {
        if let Some(plan) = frontline_registry.get_plan_mut(fl_id, country_id) {
            plan.stance = stance;
        }
    }

    fn set_controller(
        state_registry: &mut StateRegistry,
        state_id: StateId,
        controller: CountryId,
    ) {
        if let Some(state) = state_registry.states.iter_mut().find(|s| s.id == state_id) {
            state.controller_country = Some(controller);
        }
    }

    /// 要求テスト項目1: Offensive＋攻勢線ありでArmy所属Divisionが攻撃対象になる
    /// (State2[前線]→State3→State4の複数ホップ経路を実際に発行することを確認する)。
    #[test]
    fn offensive_stance_with_line_moves_army_division_toward_target() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(4)));
        assert_eq!(
            division.current_path,
            vec![StateId(3), StateId(4)],
            "複数ホップのフルパスを発行するはず(1ホップに限定しない)"
        );
        assert!(
            frontline_registry
                .frontline_generated_movements
                .contains(&division_id)
        );
    }

    /// 要求テスト項目2: Defensiveでは攻勢線が設定済みでも攻撃しない。
    #[test]
    fn defensive_stance_does_not_attack_even_with_offensive_line_set() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Defend,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "既に前線地域(State2)におり、Defendでは維持されるだけで攻撃しないはず"
        );
        assert_eq!(division.destination, None);
    }

    /// 要求テスト項目3: Stoppedでは攻勢線が設定済みでも攻撃しない。
    #[test]
    fn stopped_stance_does_not_attack_even_with_offensive_line_set() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Stopped,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert_eq!(division.destination, None);
    }

    /// 要求テスト項目4: 攻勢線が設定されていなければ、Offensiveでも(P21-006までと同じく)
    /// Army経由Divisionは攻撃対象にならない(既存挙動を維持)。
    #[test]
    fn offensive_stance_without_line_does_not_attack_army_division() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "攻勢線なしではArmy経由Divisionは(P21-006までと同じく)攻撃対象にならないはず"
        );
        assert_eq!(division.destination, None);
    }

    /// 要求テスト項目5: 直接割当されたDivisionは、同じDivisionが所属するArmyも同じ前線へ
    /// 割り当てられていても、直接割当として一度だけ処理される(優先度の確認)。
    #[test]
    fn direct_assignment_takes_priority_over_army_assignment_for_same_division() {
        let (_state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .assign_division(
                division_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();

        let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
        let resolved = resolve_effective_division_ids_for_plan(
            plan,
            &army_registry,
            &frontline_registry.division_frontline_map,
        );
        assert_eq!(
            resolved,
            vec![division_id],
            "直接割当済みDivisionはArmy経由と重複せず1件だけ解決されるはず"
        );
    }

    /// 要求テスト項目6: 同じDivisionが二重処理されない(直接割当+Army割当が重なっても、
    /// 攻撃発行は1回だけで整合した状態になる)。
    #[test]
    fn same_division_is_not_double_processed_for_attack() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .assign_division(
                division_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(3)));
        assert_eq!(
            division.current_path,
            vec![StateId(3)],
            "二重処理されていれば経路が壊れる(空になる/矛盾する)はずだが、単一の正しい経路のまま"
        );
    }

    /// 要求テスト項目8: 攻勢線がある場合、直接割当Division・Army経由Divisionの双方が
    /// 同じ攻勢線(未確保State集合)を目標として使う。
    #[test]
    fn direct_and_army_divisions_share_the_same_offensive_line_targets() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();

        let direct_division = make_test_division(0, CountryId(1), StateId(2));
        let direct_division_id = military_registry.add_division(direct_division);
        frontline_registry
            .assign_division(
                direct_division_id,
                fl_id,
                CountryId(1),
                &military_registry,
                &war_registry,
            )
            .unwrap();

        let (army_id, army_division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3), StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let line_targets = [StateId(3), StateId(4)];
        for division_id in [direct_division_id, army_division_id] {
            let division = military_registry.divisions.get(&division_id).unwrap();
            assert_eq!(division.status, DivisionStatus::Moving);
            let dest = division
                .destination
                .expect("直接割当・Army経由いずれも攻勢線目標へ移動するはず");
            assert!(
                line_targets.contains(&dest),
                "{division_id:?}の目標{dest:?}が攻勢線の集合外"
            );
        }
    }

    /// 要求テスト項目9/10: 一部が確保済みの攻勢線では、未確保のStateだけへ進む
    /// (確保済みStateは目標候補から除外される)。
    #[test]
    fn captured_offensive_line_state_is_excluded_and_remaining_state_is_targeted() {
        let (
            mut state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            fl_id,
        ) = setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3), StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        // State3を確保済み(自国支配)にする。
        set_controller(&mut state_registry, StateId(3), CountryId(1));
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        // `process_daily_frontline_plans`経由だと、State3の支配変更が前線国境
        // (attacker_front_regions/defender_front_regions)自体の再計算にも波及し、
        // 防御配置(既存・無変更の`process_defensive_plan`)が新しい国境地域への再配置を
        // 同時に行ってしまい、「攻勢線の目標選択」だけを単独で検証できない。
        // このテストの主張(確保済みStateが目標候補から除外されること)を正確に検証する
        // ため、`process_offensive_line_attack`を直接呼ぶ。
        let plan = frontline_registry
            .get_plan(fl_id, CountryId(1))
            .unwrap()
            .clone();
        process_offensive_line_attack(
            &plan,
            plan.offensive_line_region_ids.as_deref().unwrap(),
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            &army_registry,
            CountryId(1),
            CountryId(2),
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.destination,
            Some(StateId(4)),
            "確保済みのState3ではなく、未確保のState4を目標にするはず"
        );
    }

    /// 要求テスト項目11/12: 攻勢線の全Stateを確保済みなら、新規攻撃移動は発行されず、
    /// Offensive姿勢のまま維持される(攻勢線より先へは自動進軍しない)。
    #[test]
    fn fully_captured_offensive_line_generates_no_new_attack_and_keeps_offensive_stance() {
        let (
            mut state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            fl_id,
        ) = setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_controller(&mut state_registry, StateId(3), CountryId(1));
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        // 前線国境再計算との相互作用を避け、攻撃発行そのものだけを検証するため
        // `process_offensive_line_attack`を直接呼ぶ(理由は前テストと同じ)。
        let plan = frontline_registry
            .get_plan(fl_id, CountryId(1))
            .unwrap()
            .clone();
        process_offensive_line_attack(
            &plan,
            plan.offensive_line_region_ids.as_deref().unwrap(),
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            &army_registry,
            CountryId(1),
            CountryId(2),
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "全State確保済みなら新規攻撃移動を発行しないはず(State4[線の外]へも進まない)"
        );
        assert_eq!(division.destination, None);
        assert_eq!(
            frontline_registry
                .get_plan(fl_id, CountryId(1))
                .unwrap()
                .stance,
            FrontlineStance::Offensive,
            "攻勢線到達はDefendへの自動遷移を起こさないはず(objective_region_idとは異なる終端)"
        );
    }

    /// 要求テスト項目13: 到達不能な目標(State5、孤立)しかない場合は不正な移動を発行しない。
    #[test]
    fn unreachable_offensive_line_target_generates_no_movement() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(5)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert_eq!(division.destination, None);
    }

    /// 要求テスト項目14: 複数Armyが同じFrontlineの同じ攻勢線を共有しても、それぞれが
    /// 矛盾なく有効な目標(攻勢線の集合内)へ処理される(重複・競合しない)。
    #[test]
    fn multiple_armies_sharing_frontline_do_not_duplicate_processing() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army1_id, division1_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        let (army2_id, division2_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army1_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .assign_army(army2_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3), StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let line_targets = [StateId(3), StateId(4)];
        for division_id in [division1_id, division2_id] {
            let division = military_registry.divisions.get(&division_id).unwrap();
            assert_eq!(division.status, DivisionStatus::Moving);
            let dest = division.destination.unwrap();
            assert!(line_targets.contains(&dest));
        }
    }

    /// 要求テスト項目15: Armyへの加入は次回の通常処理から反映される
    /// (加入前は攻撃対象にならず、加入後の次のprocess_daily_frontline_plansから対象になる)。
    #[test]
    fn division_added_to_army_is_attacked_from_next_processing() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, _first_division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        // 新しいDivisionを作成し、既存Armyへ加入させる。
        let new_division = make_test_division(0, CountryId(1), StateId(2));
        let new_division_id = military_registry.add_division(new_division);
        army_registry
            .add_division(army_id, new_division_id, CountryId(1), &military_registry)
            .unwrap();

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&new_division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Moving,
            "加入直後の通常処理から攻撃対象に含まれるはず"
        );
        assert_eq!(division.destination, Some(StateId(3)));
    }

    /// 要求テスト項目16: Armyの前線割当を解除すると、以降の通常処理で新規攻撃を発行しない。
    #[test]
    fn unassigning_army_from_frontline_stops_new_attacks() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        frontline_registry
            .unassign_army(army_id, CountryId(1), &army_registry)
            .unwrap();

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(division.status, DivisionStatus::Idle);
        assert_eq!(division.destination, None);
    }

    /// 要求テスト項目17: Offensive(攻勢線あり)で移動中のDivisionは、Defendへ姿勢変更すると
    /// P21-006の防御配置(前線地域への再配置)へ復帰する。
    #[test]
    fn switching_to_defensive_reverts_in_flight_attack_to_defensive_placement() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );
        assert_eq!(
            military_registry
                .divisions
                .get(&division_id)
                .unwrap()
                .destination,
            Some(StateId(4)),
            "テスト前提: 攻勢線目標へ向けて移動中であること"
        );

        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Defend,
        );
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_ne!(
            division.destination,
            Some(StateId(4)),
            "Defendへ切り替えたら攻勢線目標への移動は取り消されるはず"
        );
    }

    /// 要求テスト項目18: 和平相当のFrontline削除後、前線・作戦命令・部隊割り当ての
    /// いずれにも削除済みFrontlineへの参照が残らない。
    #[test]
    fn removing_frontline_after_offensive_attack_leaves_no_dangling_references() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            None,
            &army_registry,
        );
        assert!(
            military_registry
                .divisions
                .get(&division_id)
                .unwrap()
                .destination
                .is_some(),
            "テスト前提: 攻撃移動が発行されていること"
        );

        frontline_registry.remove_frontline(fl_id, &military_registry);

        assert!(frontline_registry.frontlines.is_empty());
        assert!(frontline_registry.plans.is_empty());
        assert!(!frontline_registry.army_frontline_map.contains_key(&army_id));
        assert!(
            !frontline_registry
                .division_frontline_map
                .contains_key(&division_id)
        );
    }

    /// 要求テスト項目19: 宣戦布告当日は攻勢線が設定されていても新規移動を一切発行しない
    /// (既存の宣戦当日ガードを維持)。
    #[test]
    fn declaration_day_guard_prevents_offensive_line_attack() {
        let (state_registry, war_registry, mut military_registry, mut frontline_registry, fl_id) =
            setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(4)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        // setup_offensive_line_environment()のWar.start_dateは"1800/01/01"。
        process_daily_frontline_plans(
            &war_registry,
            &state_registry,
            &mut military_registry,
            &mut frontline_registry,
            Some("1800/01/01"),
            &army_registry,
        );

        let division = military_registry.divisions.get(&division_id).unwrap();
        assert_eq!(
            division.status,
            DivisionStatus::Idle,
            "宣戦当日は攻勢線があっても新規移動を発行しないはず"
        );
        assert_eq!(division.destination, None);
    }

    // ─── P21-008: OffensiveLineProgress(UI状態計算) ────────────────────────────

    /// `compute_offensive_line_progress`が5状態それぞれを正しく判定する。
    #[test]
    fn offensive_line_progress_reports_all_five_states() {
        let (
            mut state_registry,
            war_registry,
            mut military_registry,
            mut frontline_registry,
            fl_id,
        ) = setup_offensive_line_environment();
        let mut army_registry = ArmyRegistry::default();
        let (army_id, _division_id) =
            setup_army_for_c1(&mut military_registry, &mut army_registry, StateId(2));
        frontline_registry
            .assign_army(army_id, fl_id, CountryId(1), &army_registry, &war_registry)
            .unwrap();

        let frontline = frontline_registry.frontlines.get(&fl_id).unwrap().clone();

        // 1. 未設定
        {
            let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
            assert_eq!(
                compute_offensive_line_progress(
                    plan,
                    &frontline,
                    &state_registry,
                    &military_registry,
                    &army_registry,
                    &frontline_registry,
                ),
                OffensiveLineProgress::NotSet
            );
        }

        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(3)],
                &state_registry,
                &war_registry,
            )
            .unwrap();

        // 2. 設定済みだがOffensiveではない(準備中)
        {
            let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
            assert_eq!(
                compute_offensive_line_progress(
                    plan,
                    &frontline,
                    &state_registry,
                    &military_registry,
                    &army_registry,
                    &frontline_registry,
                ),
                OffensiveLineProgress::Preparing
            );
        }

        set_stance(
            &mut frontline_registry,
            fl_id,
            CountryId(1),
            FrontlineStance::Offensive,
        );

        // 3. Offensive・到達可能(実行中)
        {
            let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
            assert_eq!(
                compute_offensive_line_progress(
                    plan,
                    &frontline,
                    &state_registry,
                    &military_registry,
                    &army_registry,
                    &frontline_registry,
                ),
                OffensiveLineProgress::InProgress
            );
        }

        // 4. 全確保済み(到達済み)
        set_controller(&mut state_registry, StateId(3), CountryId(1));
        {
            let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
            assert_eq!(
                compute_offensive_line_progress(
                    plan,
                    &frontline,
                    &state_registry,
                    &military_registry,
                    &army_registry,
                    &frontline_registry,
                ),
                OffensiveLineProgress::Reached
            );
        }

        // 5. 到達不能(未確保だが到達可能な有効Divisionがいない)
        frontline_registry
            .set_offensive_line(
                fl_id,
                CountryId(1),
                &[StateId(5)],
                &state_registry,
                &war_registry,
            )
            .unwrap();
        {
            let plan = frontline_registry.get_plan(fl_id, CountryId(1)).unwrap();
            assert_eq!(
                compute_offensive_line_progress(
                    plan,
                    &frontline,
                    &state_registry,
                    &military_registry,
                    &army_registry,
                    &frontline_registry,
                ),
                OffensiveLineProgress::NoReachableTargets
            );
        }
    }
}
