/// 陸上戦闘データモジュール
/// 進行中の戦闘を管理する BattleRegistry と関連データ型
use crate::common::{BattleId, CountryId, DivisionId, StateId, WarId};
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
///
/// P21-siege: 複数師団による共同参戦(combined-arms)に対応するため、攻撃側・防御側とも
/// 単一`DivisionId`ではなく`Vec<DivisionId>`で複数師団を保持する。同じ州へ向かった自軍の
/// 別師団は、進行中の戦闘に(攻撃側としてのみ)後から合流できる。
/// 防御側は戦闘開始時にその州にいた敵師団を全員まとめて初期参加者とする
/// (開始後に敵の防御側援軍が合流する経路は今回のスコープ外、`process_movement`の
/// 自国領移動分岐がそもそも`process_division_arrival`を経由しないため)。
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
    /// 攻撃側ユニットID一覧(同じ州へ後から合流した師団も含む)
    pub attacker_division_ids: Vec<DivisionId>,
    /// 防御側ユニットID一覧(戦闘開始時にその州にいた敵師団全員)
    pub defender_division_ids: Vec<DivisionId>,
    /// 攻撃側各師団の出撃元地域(敗北時の退路)。合流した師団ごとに個別に記録する
    /// (`process_division_arrival`到達時点で`Division.current_state`は既に到着先へ
    /// 書き換わっているため、`current_state`からは復元できない)
    pub attacker_origins: HashMap<DivisionId, StateId>,
    /// 戦闘開始日（文字列形式）
    pub start_date: String,
    /// 経過日数
    pub elapsed_days: u32,
    /// 戦闘状態
    pub status: BattleStatus,
}

/// 全戦闘を管理するリソース
#[derive(Resource, Default, Debug)]
pub struct BattleRegistry {
    pub battles: HashMap<BattleId, Battle>,
    next_id: usize,
}

impl BattleRegistry {
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 戦闘を登録する。同じ地域に進行中の戦闘がある場合は Err を返す
    pub fn start_battle(&mut self, battle: Battle) -> Result<BattleId, &'static str> {
        // 同一地域に進行中の戦闘が既にある場合は重複登録を防ぐ
        let already_exists = self
            .battles
            .values()
            .any(|b| b.state_id == battle.state_id && b.status == BattleStatus::Ongoing);
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
