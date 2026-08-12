use crate::app::game_state::GameState;
use crate::app::time::DailySimulationSet;
use crate::building::construction::ConstructionQueueItem;
use crate::common::{CountryId, StateId};
use crate::economy::economic_state::EconomicState;
use crate::economy::resources::CountryStockpile;
use crate::politics::interest_groups::CountryPoliticsData;
use crate::politics::reform::PoliticalReform;
use crate::research::allocation::CountryResearchState;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

// ─── 統治体制 ───────────────────────────────────────────────────────────────

/// 統治体制の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GovernmentType {
    #[default]
    Monarchy,
    Republic,
    Dictatorship,
    Theocracy,
}

impl GovernmentType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            GovernmentType::Monarchy => "government.monarchy",
            GovernmentType::Republic => "government.republic",
            GovernmentType::Dictatorship => "government.dictatorship",
            GovernmentType::Theocracy => "government.theocracy",
        }
    }
}

// ─── 経済体制 ───────────────────────────────────────────────────────────────

/// 経済体制の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EconomicSystem {
    #[default]
    FreeMarket,
    Mercantilism,
    StateCapitalism,
    Socialism,
    PlannedEconomy,
    TempleEconomy,
    GuildEconomy,
}

impl EconomicSystem {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            EconomicSystem::FreeMarket => "economic_system.free_market",
            EconomicSystem::Mercantilism => "economic_system.mercantilism",
            EconomicSystem::StateCapitalism => "economic_system.state_capitalism",
            EconomicSystem::Socialism => "economic_system.socialism",
            EconomicSystem::PlannedEconomy => "economic_system.planned_economy",
            EconomicSystem::TempleEconomy => "economic_system.temple_economy",
            EconomicSystem::GuildEconomy => "economic_system.guild_economy",
        }
    }
}

// ─── 国家データ ──────────────────────────────────────────────────────────────

fn default_tax_rate() -> f32 {
    0.15
}

/// P21-001: `assets/data/countries.ron`は`available_manpower`を明示しないため、
/// 素の`#[serde(default)]`(=`u64::default()`=0)では募兵が常に不可能になってしまう
/// (募兵導線が接続されるまで顕在化していなかった潜在バグ)。`CountryData::default()`と
/// 同じ値を明示的なデフォルトとして使う。
fn default_available_manpower() -> u64 {
    100_000
}

/// 1国家分のゲームデータ（RONデシリアライズ対応）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryData {
    pub id: CountryId,
    /// 表示名（ASCII推奨）
    pub name: String,
    /// マップカラー (r, g, b) 0.0〜1.0
    pub map_color: [f32; 3],
    /// 首都州ID
    pub capital_state_id: StateId,
    /// 国庫残高（ゴールド）
    pub treasury: f64,
    /// 統治体制
    pub government_type: GovernmentType,
    /// 経済体制
    pub economic_system: EconomicSystem,

    /// 国家資源備蓄
    #[serde(default)]
    pub stockpile: CountryStockpile,
    /// 税率 (0.0〜1.0)
    #[serde(default = "default_tax_rate")]
    pub tax_rate: f32,
    /// 月次収入
    #[serde(default)]
    pub monthly_income: f64,
    /// 月次支出
    #[serde(default)]
    pub monthly_expenses: f64,
    /// 月次収支
    #[serde(default)]
    pub monthly_balance: f64,
    /// 科学研究力
    #[serde(default)]
    pub science_research_capacity: f64,
    /// 魔法研究力
    #[serde(default)]
    pub magic_research_capacity: f64,
    /// 国家経済状況 (5段階)
    #[serde(default)]
    pub economic_state: EconomicState,
    /// 建設キュー
    #[serde(default)]
    pub construction_queue: Vec<ConstructionQueueItem>,

    /// 国家の研究状態
    #[serde(default)]
    pub research_state: CountryResearchState,
    /// 政治・価値観・利益団体データ
    #[serde(default = "CountryPoliticsData::new_default")]
    pub politics: CountryPoliticsData,
    /// 進行中の価値観改革
    #[serde(default)]
    pub current_reform: Option<PoliticalReform>,

    // ── 軍事データ ────────────────────────────────────────────────────────
    /// 募集可能な人的資源
    #[serde(default = "default_available_manpower")]
    pub available_manpower: u64,
    /// 動員済みの人的資源（軍隊に所属している合計）
    #[serde(default)]
    pub mobilized_manpower: u64,
    /// 必要な軍備品の総量
    #[serde(default)]
    pub total_military_equipment_required: f64,
    /// 利用可能な軍備品の総量（備蓄から割り当てられた分）
    #[serde(default)]
    pub total_military_equipment_available: f64,
    /// 月次の軍維持費
    #[serde(default)]
    pub monthly_military_expenses: f64,
    /// 募集キュー
    #[serde(default)]
    pub recruitment_queue: Vec<crate::military::recruitment::RecruitmentQueueItem>,
}

impl Default for CountryData {
    fn default() -> Self {
        Self {
            id: CountryId(0),
            name: "Default Country".to_string(),
            map_color: [0.5, 0.5, 0.5],
            capital_state_id: StateId(0),
            treasury: 1000.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            stockpile: CountryStockpile::default(),
            tax_rate: 0.15,
            monthly_income: 0.0,
            monthly_expenses: 0.0,
            monthly_balance: 0.0,
            science_research_capacity: 10.0,
            magic_research_capacity: 10.0,
            economic_state: EconomicState::default(),
            construction_queue: Vec::new(),
            research_state: CountryResearchState::default(),
            politics: CountryPoliticsData::new_default(),
            current_reform: None,
            available_manpower: 100_000,
            mobilized_manpower: 0,
            total_military_equipment_required: 0.0,
            total_military_equipment_available: 0.0,
            monthly_military_expenses: 0.0,
            recruitment_queue: Vec::new(),
        }
    }
}

impl CountryData {
    /// bevy::Color として国家色を取得する
    pub fn bevy_color(&self) -> Color {
        Color::srgb(self.map_color[0], self.map_color[1], self.map_color[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P21-001回帰テスト: `assets/data/countries.ron`は`available_manpower`を
    /// 明示しない(実データで確認済み)。素の`#[serde(default)]`だと`u64::default()`=0に
    /// なり、募兵UIが接続されるまで発覚しなかった「全国家が常に募兵不可能」という
    /// 潜在バグの原因だった。RONデシリアライズ時に`CountryData::default()`と
    /// 同じ100_000が使われることを固定する。
    #[test]
    fn available_manpower_defaults_to_nonzero_when_absent_from_ron() {
        // countries.ron の実際のエントリ形式(available_manpower省略)を再現
        let ron = r#"(
            id: (0),
            name: "Test Country",
            map_color: (0.5, 0.5, 0.5),
            capital_state_id: (0),
            treasury: 5000.0,
            government_type: Monarchy,
            economic_system: Mercantilism,
            stockpile: (amounts: {}),
        )"#;
        let country: CountryData = ron::from_str(ron).expect("minimal country RON must parse");
        assert_eq!(country.available_manpower, 100_000);
    }
}

// ─── レジストリ ──────────────────────────────────────────────────────────────

/// ゲーム内全国家データを保持するリソース
#[derive(Resource, Default)]
pub struct CountryRegistry {
    pub countries: Vec<CountryData>,
}

impl CountryRegistry {
    /// ID で国家を検索する
    pub fn get(&self, id: CountryId) -> Option<&CountryData> {
        self.countries.iter().find(|c| c.id == id)
    }

    /// ID で国家を可変検索する
    pub fn get_mut(&mut self, id: CountryId) -> Option<&mut CountryData> {
        self.countries.iter_mut().find(|c| c.id == id)
    }
}

pub mod country_ai;

use country_ai::{CountryAiRegistry, handle_daily_country_ai};

// ─── プレイヤー国家 ──────────────────────────────────────────────────────────

/// プレイヤーが選択した国家ID（Playing 中は必ず Some）
#[derive(Resource, Default, Debug)]
pub struct PlayerCountry(pub Option<CountryId>);

// ─── プラグイン ──────────────────────────────────────────────────────────────

/// 国家関連プラグイン
pub struct CountryPlugin;

impl Plugin for CountryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CountryRegistry::default())
            .insert_resource(PlayerCountry::default())
            .insert_resource(CountryAiRegistry::default())
            .add_systems(
                Update,
                handle_daily_country_ai
                    .in_set(DailySimulationSet::CountryAi)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}
