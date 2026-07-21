use crate::building::construction::ConstructionQueueItem;
use crate::common::{CountryId, StateId};
use crate::economy::economic_state::EconomicState;
use crate::economy::resources::CountryStockpile;
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
    /// 表示用英語名
    pub fn display_name(self) -> &'static str {
        match self {
            GovernmentType::Monarchy => "Monarchy",
            GovernmentType::Republic => "Republic",
            GovernmentType::Dictatorship => "Dictatorship",
            GovernmentType::Theocracy => "Theocracy",
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
    /// 表示用英語名
    pub fn display_name(self) -> &'static str {
        match self {
            EconomicSystem::FreeMarket => "Free Market",
            EconomicSystem::Mercantilism => "Mercantilism",
            EconomicSystem::StateCapitalism => "State Capitalism",
            EconomicSystem::Socialism => "Socialism",
            EconomicSystem::PlannedEconomy => "Planned Economy",
            EconomicSystem::TempleEconomy => "Temple Economy",
            EconomicSystem::GuildEconomy => "Guild Economy",
        }
    }
}

// ─── 国家データ ──────────────────────────────────────────────────────────────

fn default_tax_rate() -> f32 {
    0.15
}

/// 1国家分のゲームデータ（RONデシリアライズ対応）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryData {
    pub id: CountryId,
    /// 表示名（ASCII推奨、ICU4X制限のため）
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
}

impl CountryData {
    /// bevy::Color として国家色を取得する
    pub fn bevy_color(&self) -> Color {
        Color::srgb(self.map_color[0], self.map_color[1], self.map_color[2])
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
            .insert_resource(PlayerCountry::default());
    }
}
