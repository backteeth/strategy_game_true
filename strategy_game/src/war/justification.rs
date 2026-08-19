use crate::common::{CountryId, DiplomaticCrisisId, StateId};
use crate::country::CountryRegistry;
use crate::diplomacy::data::{DiplomacyRegistry, TreatyType};
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 正当化に必要なデフォルト日数
pub const DEFAULT_JUSTIFICATION_DAYS: u32 = 30;

/// 戦争正当化データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarJustification {
    pub id: usize,
    pub initiator: CountryId,
    pub target: CountryId,
    pub target_state: StateId,
    pub start_date: String,
    pub required_days: u32,
    pub days_passed: u32,
    pub is_ready: bool,
    /// P21-016: この正当化を生んだCrisis(拒否・AI拒否・期限切れのいずれか)。
    /// Crisis経由でない(既存の通常正当化)場合は`None`。旧Saveには存在しない
    /// フィールドのため`#[serde(default)]`(=`None`)で後方互換を保つ。
    #[serde(default)]
    pub source_crisis_id: Option<DiplomaticCrisisId>,
    /// P21-016: 正当化がReadyになった時点で確定した攻撃側支持国(要求国自身は含まない、
    /// CountryId昇順・重複なし)。`source_crisis_id`が`None`なら常に空。
    #[serde(default)]
    pub committed_attackers: Vec<CountryId>,
    /// P21-016: 正当化がReadyになった時点で確定した防御側支持国(対象国自身は含まない、
    /// CountryId昇順・重複なし)。`source_crisis_id`が`None`なら常に空。
    #[serde(default)]
    pub committed_defenders: Vec<CountryId>,
}

#[derive(Resource, Default, Debug, Serialize, Deserialize)]
pub struct WarJustificationRegistry {
    pub justifications: HashMap<usize, WarJustification>,
    next_id: usize,
}

impl WarJustificationRegistry {
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    /// 保存されていた要素・次回ID発行値から`WarJustificationRegistry`を復元する
    /// （P21-SAVE-002D）。`crate::save`のDTO型ではなく、通常のコレクションと
    /// カウンタだけを引数にとる。
    pub(crate) fn from_saved_parts(
        justifications: HashMap<usize, WarJustification>,
        next_id: usize,
    ) -> Self {
        Self {
            justifications,
            next_id,
        }
    }

    /// 正当化が開始可能かを検証し、エラーメッセージを返す
    pub fn can_start_justification(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
    ) -> Result<(), &'static str> {
        self.can_start_justification_with_date(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
            None,
        )
    }

    /// 日付指定付きで正当化が開始可能かを検証する
    #[allow(clippy::too_many_arguments)]
    pub fn can_start_justification_with_date(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
        current_date_str: Option<&str>,
    ) -> Result<(), &'static str> {
        // P20-009: 戻り値は表示用の英語原文ではなく安定した翻訳キー(war_error.justify.*)。
        // 呼び出し元UI(diplomacy_panel)が`localization::t()`で表示直前に言語へ解決する。

        // 自国に対する正当化は不可
        if initiator == target {
            return Err("war_error.justify.self");
        }

        // 国家の存在チェック
        if country_registry.get(initiator).is_none() {
            return Err("war_error.justify.initiator_missing");
        }
        if country_registry.get(target).is_none() {
            return Err("war_error.justify.target_missing");
        }

        // 州の存在チェックと所有権確認
        let state = state_registry
            .get(target_state)
            .ok_or("war_error.justify.state_missing")?;
        if state.owner_country_id != target {
            return Err("war_error.justify.state_not_owned");
        }

        // 同盟・不可侵条約・休戦のチェック
        if let Some(rel) = diplomacy_registry.get(initiator, target) {
            if rel.has_treaty(TreatyType::Alliance) {
                return Err("war_error.justify.ally");
            }
            if rel.has_treaty(TreatyType::NonAggressionPact) {
                return Err("war_error.justify.nap");
            }
        }

        if let Some(date_str) = current_date_str {
            if diplomacy_registry.is_in_truce(initiator, target, date_str) {
                return Err("war_error.justify.truce");
            }
        } else if matches!(diplomacy_registry.get(initiator, target), Some(rel) if rel.truce_until.is_some())
        {
            return Err("war_error.justify.truce");
        }

        // 重複正当化のチェック
        let is_duplicate = self.justifications.values().any(|j| {
            j.initiator == initiator && j.target == target && j.target_state == target_state
        });
        if is_duplicate {
            return Err("war_error.justify.duplicate");
        }

        Ok(())
    }

    /// 正当化を開始する
    #[allow(clippy::too_many_arguments)]
    pub fn start_justification(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        start_date: String,
        country_registry: &CountryRegistry,
        state_registry: &StateRegistry,
        diplomacy_registry: &DiplomacyRegistry,
    ) -> Result<usize, &'static str> {
        self.can_start_justification(
            initiator,
            target,
            target_state,
            country_registry,
            state_registry,
            diplomacy_registry,
        )?;

        let id = self.next_id;
        self.next_id += 1;

        let justification = WarJustification {
            id,
            initiator,
            target,
            target_state,
            start_date,
            required_days: DEFAULT_JUSTIFICATION_DAYS,
            days_passed: 0,
            is_ready: false,
            source_crisis_id: None,
            committed_attackers: Vec::new(),
            committed_defenders: Vec::new(),
        };

        self.justifications.insert(id, justification);
        Ok(id)
    }

    /// 日次進行処理（時間連動）
    pub fn process_daily_justifications(&mut self, state_registry: &StateRegistry) {
        let mut invalid_ids = Vec::new();

        for justification in self.justifications.values_mut() {
            // 対象州の所有者が変わった等の安全チェック
            if let Some(state) = state_registry.get(justification.target_state) {
                if state.owner_country_id != justification.target {
                    invalid_ids.push(justification.id);
                    continue;
                }
            } else {
                invalid_ids.push(justification.id);
                continue;
            }

            if !justification.is_ready {
                justification.days_passed += 1;
                if justification.days_passed >= justification.required_days {
                    justification.is_ready = true;
                }
            }
        }

        // 無効になった正当化を削除
        for id in invalid_ids {
            self.justifications.remove(&id);
        }
    }

    /// 準備完了した特定正当化を取得
    pub fn get_ready_justification(
        &self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
    ) -> Option<&WarJustification> {
        self.justifications.values().find(|j| {
            j.initiator == initiator
                && j.target == target
                && j.target_state == target_state
                && j.is_ready
        })
    }

    /// P21-011: Crisisの拒否/期限切れ時に、initiatorへ即座に完成済み(is_ready=true)の
    /// WarJustificationを付与する。同じ(initiator,target,target_state)の既存正当化が
    /// あれば、それを即座に完了扱いへ引き上げる(重複作成しない)。同じCrisisに対して
    /// 誤って複数回呼ばれても安全(冪等)。戻り値は付与されたjustificationのid。
    ///
    /// P21-016: `source_crisis_id`/`committed_attackers`/`committed_defenders`は
    /// Crisis経由の支持コミットメントのスナップショット(§4)。Crisis経由でない
    /// 呼び出しは`None`/空Vecを渡す。既存の`(initiator,target,target_state)`一致で
    /// 既存正当化を「引き上げる」分岐でも、これら3フィールドを最新の呼び出し内容で
    /// 上書きする(同じ国家ペアに複数Crisisが存在した場合、最後に完了したCrisisの
    /// スナップショットを正とする — 既存の`is_ready`/`days_passed`上書きと同じ規約)。
    #[allow(clippy::too_many_arguments)]
    pub fn grant_completed_justification(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
        start_date: String,
        source_crisis_id: Option<DiplomaticCrisisId>,
        committed_attackers: Vec<CountryId>,
        committed_defenders: Vec<CountryId>,
    ) -> usize {
        if let Some(existing) = self.justifications.values_mut().find(|j| {
            j.initiator == initiator && j.target == target && j.target_state == target_state
        }) {
            existing.is_ready = true;
            existing.days_passed = existing.required_days;
            existing.source_crisis_id = source_crisis_id;
            existing.committed_attackers = committed_attackers;
            existing.committed_defenders = committed_defenders;
            return existing.id;
        }

        let id = self.next_id;
        self.next_id += 1;
        self.justifications.insert(
            id,
            WarJustification {
                id,
                initiator,
                target,
                target_state,
                start_date,
                required_days: DEFAULT_JUSTIFICATION_DAYS,
                days_passed: DEFAULT_JUSTIFICATION_DAYS,
                is_ready: true,
                source_crisis_id,
                committed_attackers,
                committed_defenders,
            },
        );
        id
    }

    /// P21-011: Crisisの撤回(withdraw)により、`grant_completed_justification`で
    /// 付与済みのWarJustificationを取り消す。既に消費/削除済みの場合は`false`を返す
    /// (冪等)。
    pub fn cancel_justification(&mut self, id: usize) -> bool {
        self.justifications.remove(&id).is_some()
    }

    /// 正当化を消費（削除）
    pub fn consume_justification(
        &mut self,
        initiator: CountryId,
        target: CountryId,
        target_state: StateId,
    ) -> bool {
        if let Some(id) = self.justifications.iter().find_map(|(&id, j)| {
            if j.initiator == initiator
                && j.target == target
                && j.target_state == target_state
                && j.is_ready
            {
                Some(id)
            } else {
                None
            }
        }) {
            self.justifications.remove(&id);
            true
        } else {
            false
        }
    }
}
