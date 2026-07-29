/// 陸上戦闘データモジュール
/// 進行中の戦闘を管理する BattleRegistry と関連データ型
use crate::common::{ArmyId, BattleId, CountryId, StateId, WarId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 戦闘の進行状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BattleStatus {
    /// 進行中
    Ongoing,
    /// 攻撃側勝利
    AttackerWon,
    /// 防御側勝利
    DefenderWon,
    /// 無効化・中止
    Cancelled,
}

/// 進行中の陸上戦闘データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Battle {
    /// 戦闘固有ID
    pub id: BattleId,
    /// 関連する戦争ID
    pub war_id: WarId,
    /// 戦闘発生地域
    pub state_id: StateId,
    /// 攻撃側国家
    pub attacker_country: CountryId,
    /// 防御側国家
    pub defender_country: CountryId,
    /// 攻撃側ユニットID
    pub attacker_army_id: ArmyId,
    /// 防御側ユニットID
    pub defender_army_id: ArmyId,
    /// 戦闘開始日（文字列形式）
    pub start_date: String,
    /// 経過日数
    pub elapsed_days: u32,
    /// 戦闘状態
    pub status: BattleStatus,
    /// 攻撃側の元の地域（敗北時の退路）
    pub attacker_origin_state: StateId,
}

/// 全戦闘を管理するリソース
#[derive(Resource, Default, Debug)]
pub struct BattleRegistry {
    pub battles: HashMap<BattleId, Battle>,
    next_id: usize,
}

impl BattleRegistry {
    /// 戦闘を登録する。同じ地域に進行中の戦闘がある場合は Err を返す
    pub fn start_battle(&mut self, battle: Battle) -> Result<BattleId, &'static str> {
        // 同一地域に進行中の戦闘が既にある場合は重複登録を防ぐ
        let already_exists = self.battles.values().any(|b| {
            b.state_id == battle.state_id
                && b.status == BattleStatus::Ongoing
        });
        if already_exists {
            return Err("Battle already ongoing in this state");
        }

        let id = BattleId(self.next_id);
        self.next_id += 1;
        let mut b = battle;
        b.id = id;
        self.battles.insert(id, b);
        Ok(id)
    }

    /// 指定地域で進行中の戦闘を取得する
    pub fn get_ongoing_battle_in_state(&self, state_id: StateId) -> Option<&Battle> {
        self.battles
            .values()
            .find(|b| b.state_id == state_id && b.status == BattleStatus::Ongoing)
    }

    /// 指定地域で進行中の戦闘（可変参照）を取得する
    pub fn get_ongoing_battle_in_state_mut(&mut self, state_id: StateId) -> Option<&mut Battle> {
        self.battles
            .values_mut()
            .find(|b| b.state_id == state_id && b.status == BattleStatus::Ongoing)
    }

    /// 終了・キャンセル済みの戦闘を除去する
    pub fn cleanup_finished_battles(&mut self) {
        self.battles
            .retain(|_, b| b.status == BattleStatus::Ongoing);
    }
}
