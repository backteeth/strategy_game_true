use crate::common::{CountryId, StateId, WarId};
use crate::diplomacy::crisis::WarGoal;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarStatus {
    /// 進行中
    Active,
    /// 攻撃側勝利
    AttackerVictory,
    /// 防御側勝利
    DefenderVictory,
    /// 白紙講和
    WhitePeace,
    /// 中止・無効化
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct War {
    pub id: WarId,
    pub name: String,
    pub attackers: HashSet<CountryId>,
    pub defenders: HashSet<CountryId>,
    /// P21-016: 攻撃側の代表国(既存の二国間中心の講和・war_score等が対象とする国)。
    /// 常に`attackers`集合に含まれる。旧Saveには存在しないフィールドなので
    /// `#[serde(default)]`(=`None`)で後方互換を保つ。旧Saveの`attackers`は常に
    /// 単一要素のみを含むため、生の値ではなく必ず`primary_attacker_id()`経由で読み、
    /// `None`の場合は集合内の最小`CountryId`へ自動的にフォールバックする(読み込み時の
    /// 個別マイグレーション処理を不要にし、どのロード経路でも常に正しい値を返す)。
    #[serde(default)]
    pub primary_attacker: Option<CountryId>,
    /// P21-016: 防御側の代表国。`primary_attacker`と同じ規約。
    #[serde(default)]
    pub primary_defender: Option<CountryId>,
    pub war_goals: Vec<WarGoal>,
    pub start_date: String,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub duration_days: u32,
    pub war_score: f32, // -100.0 (Defender winning) ~ 100.0 (Attacker winning)
    pub attacker_war_exhaustion: f32, // 0.0 ~ 100.0
    pub defender_war_exhaustion: f32, // 0.0 ~ 100.0
    pub occupied_states: HashSet<StateId>,
    pub status: WarStatus,
    #[serde(default)]
    pub winner: Option<CountryId>,
    #[serde(default)]
    pub end_reason: Option<String>,
    #[serde(default)]
    pub applied_terms: Vec<String>,

    // 戦闘結果集計用
    #[serde(default)]
    pub won_attacker_battles: u32,
    #[serde(default)]
    pub won_defender_battles: u32,
    #[serde(default)]
    pub processed_battle_ids: HashSet<crate::common::BattleId>,
}

/// P21-016: War内での陣営。多国間参加者の敵味方判定を共有APIとして提供する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarSide {
    Attacker,
    Defender,
}

impl War {
    /// 攻撃側の代表国を返す。`primary_attacker`が設定されていればそれを、
    /// 未設定(旧Save)なら`attackers`集合内の最小`CountryId`を返す
    /// (旧Saveの`attackers`は常に単一要素なので一意に定まる)。
    pub fn primary_attacker_id(&self) -> CountryId {
        self.primary_attacker.unwrap_or_else(|| {
            self.attackers
                .iter()
                .copied()
                .min_by_key(|c| c.0)
                .unwrap_or(CountryId(0))
        })
    }

    /// 防御側の代表国を返す。`primary_attacker_id`と同じ規約。
    pub fn primary_defender_id(&self) -> CountryId {
        self.primary_defender.unwrap_or_else(|| {
            self.defenders
                .iter()
                .copied()
                .min_by_key(|c| c.0)
                .unwrap_or(CountryId(0))
        })
    }

    /// 攻撃側参加国を`CountryId`昇順でソートして返す(Save出力・UI表示・決定的な
    /// テスト用。内部ストレージの`HashSet`をソート型へ変更せずに順序を保証する)。
    pub fn sorted_attackers(&self) -> Vec<CountryId> {
        let mut v: Vec<CountryId> = self.attackers.iter().copied().collect();
        v.sort_by_key(|c| c.0);
        v
    }

    /// 防御側参加国を`CountryId`昇順でソートして返す。
    pub fn sorted_defenders(&self) -> Vec<CountryId> {
        let mut v: Vec<CountryId> = self.defenders.iter().copied().collect();
        v.sort_by_key(|c| c.0);
        v
    }

    /// `country`がこのWarのどちら側の参加者かを返す。非参加国は`None`。
    pub fn side_of(&self, country: CountryId) -> Option<WarSide> {
        if self.attackers.contains(&country) {
            Some(WarSide::Attacker)
        } else if self.defenders.contains(&country) {
            Some(WarSide::Defender)
        } else {
            None
        }
    }

    /// `country`がこのWarの参加国(攻撃側・防御側いずれか)かどうか。
    pub fn is_participant(&self, country: CountryId) -> bool {
        self.side_of(country).is_some()
    }

    /// `a`と`b`がこのWarで互いに敵対する陣営に属しているかどうか
    /// (両者ともこのWarの参加国で、かつ別陣営の場合のみ`true`)。
    pub fn are_opponents(&self, a: CountryId, b: CountryId) -> bool {
        match (self.side_of(a), self.side_of(b)) {
            (Some(sa), Some(sb)) => sa != sb,
            _ => false,
        }
    }

    /// `country`から見た敵対陣営の参加国一覧を`CountryId`昇順で返す。
    /// `country`がこのWarの参加国でなければ空。
    pub fn opponents_of(&self, country: CountryId) -> Vec<CountryId> {
        match self.side_of(country) {
            Some(WarSide::Attacker) => self.sorted_defenders(),
            Some(WarSide::Defender) => self.sorted_attackers(),
            None => Vec::new(),
        }
    }
}

#[derive(Resource, Default, Debug)]
pub struct WarRegistry {
    pub wars: HashMap<WarId, War>,
    next_id: usize,
}

impl WarRegistry {
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 保存されていた要素・次回ID発行値から`WarRegistry`を復元する（P21-SAVE-002D）。
    /// `crate::save`のDTO型ではなく、通常のコレクションとカウンタだけを引数にとる。
    pub(crate) fn from_saved_parts(wars: HashMap<WarId, War>, next_id: usize) -> Self {
        Self { wars, next_id }
    }

    pub fn add_war(&mut self, mut war: War) -> WarId {
        war.id = WarId(self.next_id);
        self.next_id += 1;
        self.wars.insert(war.id, war.clone());
        war.id
    }

    pub fn get_active_war_for_country(&self, country: CountryId) -> Option<&War> {
        self.wars.values().find(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&country) || w.defenders.contains(&country))
        })
    }

    pub fn are_countries_at_war(&self, c1: CountryId, c2: CountryId) -> bool {
        if c1 == c2 {
            return false;
        }
        self.wars.values().any(|w| {
            w.status == WarStatus::Active
                && ((w.attackers.contains(&c1) && w.defenders.contains(&c2))
                    || (w.attackers.contains(&c2) && w.defenders.contains(&c1)))
        })
    }

    /// P21-016: `country`がいずれかのActive Warに参加しているかどうか
    /// (攻撃側・防御側を問わない、多国間参加者も対象)。
    pub fn is_country_at_war(&self, country: CountryId) -> bool {
        self.wars
            .values()
            .any(|w| w.status == WarStatus::Active && w.is_participant(country))
    }

    /// P21-016: `country`が参加しているActive Warの`WarId`一覧を昇順で返す。
    pub fn wars_for_country(&self, country: CountryId) -> Vec<WarId> {
        let mut ids: Vec<WarId> = self
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active && w.is_participant(country))
            .map(|w| w.id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// 宣戦布告の前提条件を検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_declare_war(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &crate::war::justification::WarJustificationRegistry,
    ) -> Result<(), &'static str> {
        self.can_declare_war_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            justification_registry,
            None,
        )
    }

    /// 日付情報を含めて宣戦布告の前提条件を検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_declare_war_with_date(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &crate::war::justification::WarJustificationRegistry,
        current_date_str: Option<&str>,
    ) -> Result<(), &'static str> {
        // P20-009: 戻り値は表示用の英語原文ではなく安定した翻訳キー(war_error.declare.*)。
        // 呼び出し元UI(diplomacy_panel)が`localization::t()`で表示直前に言語へ解決する。
        if initiator == target {
            return Err("war_error.declare.self");
        }

        if country_registry.get(initiator).is_none() {
            return Err("war_error.declare.initiator_missing");
        }
        if country_registry.get(target).is_none() {
            return Err("war_error.declare.target_missing");
        }

        let state = state_registry
            .get(target_state)
            .ok_or("war_error.declare.state_missing")?;
        if state.owner_country_id != target {
            return Err("war_error.declare.state_not_owned");
        }

        if let Some(rel) = diplomacy_registry.get(initiator, target) {
            if rel.has_treaty(crate::diplomacy::data::TreatyType::Alliance) {
                return Err("war_error.declare.ally");
            }
            if rel.has_treaty(crate::diplomacy::data::TreatyType::NonAggressionPact) {
                return Err("war_error.declare.nap");
            }
        }

        // 休戦チェック (日付指定があれば期限切れかどうか検証)
        if let Some(date_str) = current_date_str {
            if diplomacy_registry.is_in_truce(initiator, target, date_str) {
                return Err("war_error.declare.truce");
            }
        } else if matches!(diplomacy_registry.get(initiator, target), Some(rel) if rel.truce_until.is_some())
        {
            return Err("war_error.declare.truce");
        }

        if self.are_countries_at_war(initiator, target) {
            return Err("war_error.declare.already_at_war");
        }

        if justification_registry
            .get_ready_justification(initiator, target, target_state)
            .is_none()
        {
            return Err("war_error.declare.no_justification");
        }

        Ok(())
    }

    /// 宣戦布告を実行し、新しい戦争データを作成する
    #[allow(clippy::too_many_arguments)]
    pub fn declare_war(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        start_date: String,
        country_registry: &crate::country::CountryRegistry,
        state_registry: &crate::state::data::StateRegistry,
        diplomacy_registry: &mut crate::diplomacy::data::DiplomacyRegistry,
        justification_registry: &mut crate::war::justification::WarJustificationRegistry,
    ) -> Result<WarId, &'static str> {
        self.can_declare_war_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            justification_registry,
            Some(&start_date),
        )?;

        // P21-016: 正当化を消費する前に、Crisis由来の支持コミットメント・スナップショット
        // (正当化がReadyになった時点で確定済み、`WarJustificationRegistry::get_ready_justification`が
        // 返す借用の生存中に読み取る)を読む。CrisisRegistryへの再問い合わせは行わない
        // (どのCrisisに由来するか国ペアから逆引きする曖昧さを避けるため)。
        let (committed_attackers, committed_defenders) = {
            let justification = justification_registry
                .get_ready_justification(initiator, target, target_state)
                .ok_or("war_error.declare.no_justification")?;
            (
                justification.committed_attackers.clone(),
                justification.committed_defenders.clone(),
            )
        };

        // 同じ国が両陣営のコミットメントに同時に現れる、または要求国/対象国自身の
        // コミットメントと矛盾する場合は、データ破損とみなしWar全体をアトミックに拒否する
        // (正当化消費・外交関係変更のいずれも行わない)。
        let attacker_supporters: HashSet<CountryId> = committed_attackers.iter().copied().collect();
        let defender_supporters: HashSet<CountryId> = committed_defenders.iter().copied().collect();
        if attacker_supporters
            .intersection(&defender_supporters)
            .next()
            .is_some()
            || attacker_supporters.contains(&target)
            || defender_supporters.contains(&initiator)
        {
            return Err("war_error.declare.corrupt_support_snapshot");
        }

        // 正当化を消費
        justification_registry.consume_justification(initiator, target, target_state);

        // 外交関係値の悪化 (-50.0)
        if let Some(rel) = diplomacy_registry.get_or_create_mut(initiator, target) {
            rel.opinion = (rel.opinion - 50.0).clamp(-100.0, 100.0);
        }

        let attacker_name = country_registry
            .get(initiator)
            .map(|c| c.name.as_str())
            .unwrap_or("Attacker");
        let defender_name = country_registry
            .get(target)
            .map(|c| c.name.as_str())
            .unwrap_or("Defender");
        let state_name = state_registry
            .get(target_state)
            .map(|s| s.name.as_str())
            .unwrap_or("State");

        // P21-016: 確定した支持国のうち、宣戦布告時点で実在する国のみを参加させる
        // (存在しない支持国は個別に静かに除外する。集合単位の全体拒否とは別の規則)。
        let mut attackers = HashSet::new();
        attackers.insert(initiator);
        for supporter in committed_attackers {
            if country_registry.get(supporter).is_some() {
                attackers.insert(supporter);
            }
        }

        let mut defenders = HashSet::new();
        defenders.insert(target);
        for supporter in committed_defenders {
            if country_registry.get(supporter).is_some() {
                defenders.insert(supporter);
            }
        }

        let war_goal = crate::diplomacy::crisis::WarGoal {
            attacker: initiator,
            defender: target,
            goal_type: crate::diplomacy::crisis::WarGoalType::ConquerState,
            target_states: vec![target_state],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        };

        let war = War {
            id: WarId(self.next_id),
            name: format!(
                "{} Conquest of {} ({})",
                attacker_name, state_name, defender_name
            ),
            attackers,
            defenders,
            primary_attacker: Some(initiator),
            primary_defender: Some(target),
            war_goals: vec![war_goal],
            start_date,
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

        let war_id = self.add_war(war);
        Ok(war_id)
    }
}
