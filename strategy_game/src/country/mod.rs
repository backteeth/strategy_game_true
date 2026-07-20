use crate::common::{CountryId, StateId};
/// 国家データモジュール
/// CountryData、統治/経済体制enum、国家レジストリを定義する
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
}

impl CountryData {
    /// bevy::Color として国家色を取得する
    pub fn bevy_color(&self) -> Color {
        Color::srgb(self.map_color[0], self.map_color[1], self.map_color[2])
    }
}

// ─── レジストリ ──────────────────────────────────────────────────────────────

/// ゲーム内全国家データを保持するリソース
/// O(n) 検索だが、国家数は通常100以下なので問題ない
#[derive(Resource, Default)]
pub struct CountryRegistry {
    pub countries: Vec<CountryData>,
}

impl CountryRegistry {
    /// ID で国家を検索する
    pub fn get(&self, id: CountryId) -> Option<&CountryData> {
        self.countries.iter().find(|c| c.id == id)
    }
}

// ─── プレイヤー国家 ──────────────────────────────────────────────────────────

/// プレイヤーが選択した国家ID（Playing 中は必ず Some）
#[derive(Resource, Default, Debug)]
pub struct PlayerCountry(pub Option<CountryId>);

// ─── プラグイン ──────────────────────────────────────────────────────────────

/// 国家関連プラグイン
/// データはローダー側で注入するため、ここでは Resource だけ登録する
pub struct CountryPlugin;

impl Plugin for CountryPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CountryRegistry::default())
            .insert_resource(PlayerCountry::default());
    }
}
