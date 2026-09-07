use crate::common::{BattleId, CountryId, DivisionDefinitionId, DivisionId, StateId};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivisionType {
    Infantry,
    Artillery,
    Mage,
}

impl DivisionType {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            DivisionType::Infantry => "division_type.infantry",
            DivisionType::Artillery => "division_type.artillery",
            DivisionType::Mage => "division_type.mage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivisionSize {
    Light,
    Standard,
    Heavy,
}

impl DivisionSize {
    /// 表示用の翻訳キー(P20-009)。現状はUIから未参照だが、将来の表示追加に備えて維持する。
    pub fn display_name(self) -> &'static str {
        match self {
            DivisionSize::Light => "division_size.light",
            DivisionSize::Standard => "division_size.standard",
            DivisionSize::Heavy => "division_size.heavy",
        }
    }
}

/// P23-009: 全ゲームで唯一の標準Infantry Division定義ID。New Game初期配置
/// (`app::loader::spawn_debug_divisions`)・プレイヤーUIの標準Infantry募兵
/// (`ui::military_panel::RECRUIT_DIVISION_ID`)・AI月次募兵(`military::ai`)が
/// 共通して参照する、この1箇所だけが正規の値である(別々の文字列・数値として
/// 重複ハードコードしない)。
pub const STANDARD_INFANTRY_DEFINITION_ID: DivisionDefinitionId = DivisionDefinitionId(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivisionDefinition {
    pub id: DivisionDefinitionId,
    pub name: String,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub required_manpower: u64,
    pub required_equipment: f64,
    pub recruitment_days: u32,
    pub movement_speed: f32,
    pub attack: f32,
    pub defense: f32,
    pub breakthrough: f32,
    pub organization: f32,
    pub morale: f32,
    pub supply_usage: f32,
    pub maintenance_cost: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DivisionStatus {
    #[default]
    Idle,
    Moving,
    Fighting,
    Retreating,
    Occupying,
    Disbanding,
    Destroyed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Division {
    pub id: DivisionId,
    pub owner: CountryId,
    pub division_type: DivisionType,
    pub size: DivisionSize,
    pub current_state: StateId,
    #[serde(default)]
    pub destination: Option<StateId>,
    #[serde(default)]
    pub current_path: Vec<StateId>,
    pub target_state: Option<StateId>,
    pub manpower: u64,
    pub max_manpower: u64,
    pub equipment: f64,
    pub max_equipment: f64,
    pub organization: f32,
    pub max_organization: f32,
    pub morale: f32,
    pub max_morale: f32,
    pub experience: f32,
    pub supply_ratio: f32,
    pub movement_progress: f32,
    pub status: DivisionStatus,
    pub def_id: DivisionDefinitionId,
    /// 攻撃力（整数、決定的な戦闘計算に使用）
    #[serde(default = "default_attack_power")]
    pub attack_power: i32,
    /// 防御力（整数）
    #[serde(default = "default_defense_power")]
    pub defense_power: i32,
    /// 参加中の戦闘ID
    #[serde(default)]
    pub combat_id: Option<BattleId>,
}

#[derive(Resource, Default, Debug)]
pub struct MilitaryRegistry {
    pub definitions: HashMap<DivisionDefinitionId, DivisionDefinition>,
    pub divisions: HashMap<DivisionId, Division>,
    next_division_id: usize,
    /// P26-004: Divisionごとの、その日確定した日次補給充足率(`0.0..=1.0`)。
    ///
    /// 意図的に`Division`構造体自体へフィールドを追加しない――`Division`は
    /// `tests/land_war_combat_peace_test.rs`(変更禁止)を含む数十箇所で全フィールド
    /// 列挙のリテラルとして直接構築されており、新規必須フィールドを追加すると
    /// このファイルが変更なしにはコンパイルできなくなる。代わりに`ArmyRegistry.
    /// division_army_map`と同じ「DivisionId をキーにした補助map」設計を踏襲する。
    ///
    /// キーが存在しないDivisionId(まだ一度も日次補給処理を経ていない・
    /// 旧Save・存在しないID)は`supply_fulfillment_of`経由で`1.0`として扱う
    /// (spec §8: 旧SaveのdefaultはRustの`f64::default()`(0.0)ではなく1.0でなければ
    /// ならない――空mapからの欠落キー参照が自然にこの意味論を実現する)。
    pub division_supply_fulfillment: HashMap<DivisionId, f64>,
}

impl MilitaryRegistry {
    pub fn next_division_id(&self) -> usize {
        self.next_division_id
    }

    /// 保存されていたDivision・次回ID発行値・Division補給充足率から`MilitaryRegistry`を
    /// 復元する（P21-SAVE-002D、P26-004で`division_supply_fulfillment`を追加）。
    /// `definitions`(静的師団定義)は引数で明示的に受け取る(セーブには含まれないため、
    /// 呼び出し側が適用先Worldの現在値を渡す想定)。`crate::save`のDTO型ではなく、
    /// 通常のコレクションとカウンタだけを引数にとる。
    pub(crate) fn from_saved_parts(
        definitions: HashMap<DivisionDefinitionId, DivisionDefinition>,
        divisions: HashMap<DivisionId, Division>,
        next_division_id: usize,
        division_supply_fulfillment: HashMap<DivisionId, f64>,
    ) -> Self {
        Self {
            definitions,
            divisions,
            next_division_id,
            division_supply_fulfillment,
        }
    }

    /// Divisionの現在の日次補給充足率を取得する(spec §8)。未確定・未対象の
    /// DivisionIdは`1.0`(常に完全充足)を返す――需要0のDivision・まだ日次補給処理を
    /// 経ていない新規Division・旧Saveからロードした直後のDivision、いずれもこの
    /// defaultで安全にカバーされる。
    pub fn supply_fulfillment_of(&self, id: DivisionId) -> f64 {
        self.division_supply_fulfillment.get(&id).copied().unwrap_or(1.0)
    }

    pub fn add_division(&mut self, mut division: Division) -> DivisionId {
        division.id = DivisionId(self.next_division_id);
        self.next_division_id += 1;
        self.divisions.insert(division.id, division.clone());
        division.id
    }

    pub fn remove_division(&mut self, id: DivisionId) {
        self.divisions.remove(&id);
        // P26-004: 除去されたDivisionの補給充足率エントリも一緒に消す(DivisionIdは
        // 再利用されないため、放置しても実害はないが、際限なく増え続ける迷子データを
        // 残さないための衛生措置)。
        self.division_supply_fulfillment.remove(&id);
    }

    pub fn get_divisions_in_state(&self, state: StateId) -> Vec<&Division> {
        self.divisions
            .values()
            .filter(|a| a.current_state == state)
            .collect()
    }
}

fn default_attack_power() -> i32 {
    10
}

fn default_defense_power() -> i32 {
    10
}
