use crate::common::CountryId;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 順序に依存しない二国間ペアキー (A < B を常に保証)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiplomaticPairKey(pub CountryId, pub CountryId);

impl DiplomaticPairKey {
    pub fn new(a: CountryId, b: CountryId) -> Option<Self> {
        if a == b {
            None
        } else if a.0 < b.0 {
            Some(DiplomaticPairKey(a, b))
        } else {
            Some(DiplomaticPairKey(b, a))
        }
    }
}

/// 条約種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TreatyType {
    /// 不可侵条約
    NonAggressionPact,
    /// 同盟
    Alliance,
}

impl TreatyType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            TreatyType::NonAggressionPact => "treaty.non_aggression_pact",
            TreatyType::Alliance => "treaty.alliance",
        }
    }
}

/// 締結中の条約
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTreaty {
    pub treaty_type: TreatyType,
    pub countries: (CountryId, CountryId),
    pub signed_date: String,
    pub is_active: bool,
}

/// 外交活動の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiplomaticActivityType {
    ImproveRelations,
    HarmRelations,
}

impl DiplomaticActivityType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            DiplomaticActivityType::ImproveRelations => "diplomatic_activity.improve_relations",
            DiplomaticActivityType::HarmRelations => "diplomatic_activity.harm_relations",
        }
    }
}

/// 実行中の継続外交活動
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveDiplomaticActivity {
    pub activity_type: DiplomaticActivityType,
    pub initiator: CountryId,
    pub target: CountryId,
    pub days_remaining: u32,
    pub daily_opinion_change: f32,
}

/// 関係値補正内訳
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpinionModifier {
    pub label: String,
    pub value: f32,
}

/// 二国間外交関係データ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiplomaticRelation {
    pub opinion: f32, // -100.0 〜 100.0
    #[serde(default)]
    pub tension: f32, // 0.0 〜 100.0
    #[serde(default = "default_trust")]
    pub trust: f32, // 0.0 〜 100.0
    #[serde(default)]
    pub threat: f32, // 0.0 〜 100.0
    #[serde(default)]
    pub at_war: bool,
    #[serde(default)]
    pub has_military_access: bool,
    #[serde(default)]
    pub truce_until: Option<String>,

    pub treaties: Vec<ActiveTreaty>,
    pub cooldowns: HashMap<CountryId, u32>,
    pub active_activity: Option<ActiveDiplomaticActivity>,
    pub last_updated_date: String,
}

fn default_trust() -> f32 {
    50.0
}

impl Default for DiplomaticRelation {
    fn default() -> Self {
        Self {
            opinion: 0.0,
            tension: 0.0,
            trust: 50.0,
            threat: 0.0,
            at_war: false,
            has_military_access: false,
            truce_until: None,
            treaties: Vec::new(),
            cooldowns: HashMap::new(),
            active_activity: None,
            last_updated_date: "1800/01/01".to_string(),
        }
    }
}

impl DiplomaticRelation {
    pub fn clamp_opinion(&mut self) {
        self.opinion = self.opinion.clamp(-100.0, 100.0);
        self.tension = self.tension.clamp(0.0, 100.0);
        self.trust = self.trust.clamp(0.0, 100.0);
        self.threat = self.threat.clamp(0.0, 100.0);
    }

    pub fn has_treaty(&self, treaty_type: TreatyType) -> bool {
        self.treaties
            .iter()
            .any(|t| t.is_active && t.treaty_type == treaty_type)
    }

    pub fn remove_treaty(&mut self, treaty_type: TreatyType) -> bool {
        let before_len = self.treaties.len();
        self.treaties
            .retain(|t| !(t.is_active && t.treaty_type == treaty_type));
        self.treaties.len() < before_len
    }
}

/// 全二国間関係を管理するゲームリソース
#[derive(Resource, Default, Debug)]
pub struct DiplomacyRegistry {
    pub relations: HashMap<DiplomaticPairKey, DiplomaticRelation>,
}

impl DiplomacyRegistry {
    pub fn get(&self, a: CountryId, b: CountryId) -> Option<&DiplomaticRelation> {
        let key = DiplomaticPairKey::new(a, b)?;
        self.relations.get(&key)
    }

    pub fn get_or_default(&self, a: CountryId, b: CountryId) -> DiplomaticRelation {
        let key = match DiplomaticPairKey::new(a, b) {
            Some(k) => k,
            None => return DiplomaticRelation::default(),
        };
        self.relations.get(&key).cloned().unwrap_or_default()
    }

    pub fn get_mut(&mut self, a: CountryId, b: CountryId) -> Option<&mut DiplomaticRelation> {
        let key = DiplomaticPairKey::new(a, b)?;
        self.relations.get_mut(&key)
    }

    pub fn get_or_create_mut(
        &mut self,
        a: CountryId,
        b: CountryId,
    ) -> Option<&mut DiplomaticRelation> {
        let key = DiplomaticPairKey::new(a, b)?;
        Some(self.relations.entry(key).or_default())
    }

    /// 休戦を設定する
    pub fn set_truce(&mut self, a: CountryId, b: CountryId, truce_until: String) {
        if let Some(rel) = self.get_or_create_mut(a, b) {
            rel.truce_until = Some(truce_until);
        }
    }

    /// 現在の日付と照合して休戦中か判定し、有効な休戦終了日を返す
    pub fn get_truce_until(
        &self,
        a: CountryId,
        b: CountryId,
        current_date: &str,
    ) -> Option<String> {
        let rel = self.get(a, b)?;
        let truce_until_str = rel.truce_until.as_ref()?;

        let curr = crate::app::time::GameDate::from_string(current_date)?;
        let until = crate::app::time::GameDate::from_string(truce_until_str)?;

        if curr.is_at_least(&until) {
            None
        } else {
            Some(truce_until_str.clone())
        }
    }

    /// 休戦中か判定する
    pub fn is_in_truce(&self, a: CountryId, b: CountryId, current_date: &str) -> bool {
        self.get_truce_until(a, b, current_date).is_some()
    }
}

/// RONシリアライズ用初期関係定義
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitialDiplomaticRelation {
    pub country_a: CountryId,
    pub country_b: CountryId,
    pub opinion: f32,
    #[serde(default)]
    pub tension: f32,
    #[serde(default = "default_trust")]
    pub trust: f32,
    #[serde(default)]
    pub treaties: Vec<TreatyType>,
    #[serde(default)]
    pub has_military_access: bool,
    #[serde(default)]
    pub alliance: bool,
}
