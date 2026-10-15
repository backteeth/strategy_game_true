/// P21-SAVE-002A: セーブファイルのルートDTOと補助DTOの定義。
///
/// ここで定義する型はすべて保存専用のデータ構造であり、Bevyの`Resource`ではない
/// (Bevyへの依存はゼロ: このファイルは`bevy`を一切importしていない)。
/// ランタイムのBevy World/Resourceを直接シリアライズする設計は禁止されているため
/// (`verification_logs/phase-21/p21-save-001`調査報告書を参照)、`SaveGameV1`を
/// 明示的なルートDTOとして経由させる。
///
/// 型の再利用方針:
/// - 既にSerde対応済みの純粋なデータ要素(`CountryData`/`StateData`/`Division`/`War`/
///   `Battle`/`Army`/`Frontline`/`FrontlinePlan`/`DiplomaticRelation`/`WarJustification`/
///   `TerritorialClaim`/`DiplomaticCrisis`)はそのまま再利用する。
/// - Registryのコンテナ部分(`HashMap`本体・次回ID発行値)は、ランタイムのprivateフィールドへ
///   一切アクセスせずに独立定義した`Saved*`型として表現する。
/// - `CountryAiState`/`MilitaryAiState`はそのまま再利用せず、専用の`SavedCountryAiState`/
///   `SavedMilitaryAiState`を定義する(P21-SAVE-002A1: ランタイムの`dirty: bool`は
///   キャッシュ無効化・再評価要求だけを表す派生状態であり、ゲーム結果の継続に必要な
///   正規状態ではないと実コード調査で確認したため、保存対象から除外する。詳細は
///   `SavedCountryAiState`/`SavedMilitaryAiState`のdocコメント、および
///   `verification_logs/phase-21/p21-save-002a1`の完了報告書を参照)。
///
/// このラウンドではDTO型の定義のみを行う。ResourceからDTOへの変換・DTOからResourceへの
/// 復元・参照整合性検証・ファイルI/Oはいずれも実装しない(P21-SAVE-002B以降で扱う)。
use crate::common::{
    ArmyId, BattleId, ClaimId, CountryId, DiplomaticCrisisId, DivisionId, FrontlineId, WarId,
};
use crate::country::CountryData;
use crate::country::country_ai::{CountryAiDecisionReason, CountryAiMode};
use crate::diplomacy::claims::TerritorialClaim;
use crate::diplomacy::crisis::DiplomaticCrisis;
use crate::diplomacy::data::{DiplomaticPairKey, DiplomaticRelation};
use crate::military::army::Army;
use crate::military::battle::Battle;
use crate::military::data::Division;
use crate::research::data::WorldStage;
use crate::state::data::StateData;
use crate::war::data::War;
use crate::war::frontline::{Frontline, FrontlinePlan};
use crate::war::justification::WarJustification;
use crate::war::military_ai::MilitaryAiDecisionReason;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 現在のセーブ形式バージョン。
pub const SAVE_FORMAT_VERSION_V1: u32 = 1;

/// セーブファイルのルートDTO。
///
/// `version`には`#[serde(default)]`を付けない: バージョンフィールドが存在しないRONを
/// 暗黙にV1として解釈することを禁止するため(欠落時は`Deserialize`がエラーを返す。
/// `tests::deserialize_fails_when_version_field_is_missing`で確認)。
///
/// 意図的に`Default`を実装しない: 必須の正規データが欠けた不完全な値を
/// `SaveGameV1::default()`のような1行で簡単に作れてしまう構造を避けるため。
///
/// 意図的に`PartialEq`を実装しない: `countries`/`states`/`military`/`armies`/`wars`/
/// `battles`/`frontlines`等が参照する既存の純粋データ型(`CountryData`/`StateData`/
/// `Division`/`Army`/`War`/`Battle`/`Frontline`/`FrontlinePlan`/`DiplomaticRelation`/
/// `WarJustification`/`TerritorialClaim`/`DiplomaticCrisis`)は、いずれも現在`PartialEq`を
/// 実装していない(内部に`CountryStockpile`/`CountryPoliticsData`等、さらに`PartialEq`
/// 非対応の型を含むため、追加するにはこのDTOモジュール外の既存ファイルへ広範囲に
/// 手を入れる必要がある。P21-SAVE-002A1で改めてこの制約を確認し、セーブテストのためだけに
/// 既存コア型へ`PartialEq`を追加しない方針を再確認した)。今回もDTO専用モジュールの
/// 新設のみに変更範囲を限定するため、`SaveGameV1`全体への機械的な`#[derive(PartialEq)]`
/// は行わない。往復テストでの意味的等価性は、フィールド単位(`SavedGameDate`/
/// `SavedWorldCivilizationState`/`SavedCountryAiState`/`SavedMilitaryAiState`等、
/// 既にPartialEq化できる型は`==`、それ以外はキー引きした要素の個別フィールド比較)で
/// 検証する(`dto.rs`末尾の`tests`モジュールを参照)。なお`SavedCountryAiRegistry`/
/// `SavedMilitaryAiRegistry`自体は、内部の要素型をランタイムの`CountryAiState`/
/// `MilitaryAiState`から専用DTOへ切り替えたことで(P21-SAVE-002A1)、`PartialEq`が
/// 実装できるようになっている。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveGameV1 {
    pub version: u32,

    // ── 世界と進行 ──────────────────────────────────────────────
    pub date: SavedGameDate,
    pub game_speed: u8,
    pub player_country: Option<CountryId>,
    pub world_civilization: SavedWorldCivilizationState,

    // ── 国家・州 ────────────────────────────────────────────────
    pub countries: Vec<CountryData>,
    pub states: Vec<StateData>,

    // ── 外交・戦争 ──────────────────────────────────────────────
    pub diplomacy: SavedDiplomacyRegistry,
    pub war_justifications: SavedWarJustificationRegistry,
    pub wars: SavedWarRegistry,
    pub claims: SavedClaimRegistry,
    pub crises: SavedCrisisRegistry,
    pub country_ai: SavedCountryAiRegistry,
    pub military_ai: SavedMilitaryAiRegistry,

    // ── Division・Army・Battle ────────────────────────────────
    pub military: SavedMilitaryRegistry,
    pub battles: SavedBattleRegistry,
    pub armies: SavedArmyRegistry,

    // ── 前線 ────────────────────────────────────────────────────
    pub frontlines: SavedFrontlineRegistry,
}

/// `app::time::GameDate`相当のDTO。ランタイム側の`accumulator`はprivateフィールドだが、
/// このDTOはランタイムの`GameDate`から独立して定義するため、ここでは通常のpubフィールドとして
/// 表現できる(P21-SAVE-002A: ランタイム→DTO変換はまだ実装しないため、privateフィールドへ
/// アクセスする必要が生じない)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedGameDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub accumulator: f64,
}

/// `research::world_stage::WorldCivilizationState`の可変部分のみのDTO。
/// `stage_definitions`(静的マスターデータ、起動時にRONから再構築される)は含めない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWorldCivilizationState {
    pub current_stage: WorldStage,
    pub milestone_countries: HashMap<WorldStage, HashSet<usize>>,
    pub last_advanced_date: String,
}

/// `diplomacy::data::DiplomacyRegistry`のコンテナDTO。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedDiplomacyRegistry {
    pub relations: HashMap<DiplomaticPairKey, DiplomaticRelation>,
}

/// `war::justification::WarJustificationRegistry`のコンテナDTO。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedWarJustificationRegistry {
    pub justifications: HashMap<usize, WarJustification>,
    pub next_id: usize,
}

/// `war::data::WarRegistry`のコンテナDTO。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedWarRegistry {
    pub wars: HashMap<WarId, War>,
    pub next_id: usize,
    /// P29-003: 停滞した AI 戦争の白紙講和のための、停滞期間の計測状態。追加フィールド
    /// (旧 Save には存在せず、`#[serde(default)]`により空として復元する。その場合、停滞期間は
    /// ロード後の最初の評価日から計測し直す)。
    #[serde(default)]
    pub stalemate: HashMap<WarId, crate::war::stalemate::WarStalemateRecord>,
}

/// `diplomacy::claims::ClaimRegistry`のコンテナDTO。
///
/// P21-SAVE-002A決定: 実プレイでは常に空(`add_claim`は本番コード経路から一度も
/// 呼ばれない、P21-SAVE-001調査で確認済み)であっても、可変な世界状態として
/// V1へ含める(省略しない)。次回ID発行値も欠落させない。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedClaimRegistry {
    pub claims: HashMap<ClaimId, TerritorialClaim>,
    pub next_id: usize,
}

/// `diplomacy::crisis::CrisisRegistry`のコンテナDTO。`SavedClaimRegistry`と同じ理由でV1へ含める。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedCrisisRegistry {
    pub crises: HashMap<DiplomaticCrisisId, DiplomaticCrisis>,
    pub next_id: usize,
}

/// `country::country_ai::CountryAiState`の正規状態DTO。
///
/// P21-SAVE-002A1では`dirty`を「純粋なキャッシュ無効化フラグ、いつ再評価するかの
/// タイミングにしか影響しない」と判断して保存対象から除外していたが、この判断は
/// `war::update::mark_ai_registries_dirty_daily`(当時の実装)が`DayChangedMessage`の
/// たびに全国家を無条件で`dirty=true`へ戻していたため「dirtyの値自体はロード直後
/// 何であっても、その日のうちにどうせtrueへ上書きされる」という事実の上に成り立って
/// いた(P26-008-PRE-001で実測確認)。P26-008-PRE-002でこの無条件上書きを
/// イベント駆動(`war::ai_change_tracking`)へ置き換えた結果、`dirty`は初めて
/// **意味を持つ正規状態**になった――ロード直後に`dirty`を常に`true`へ戻すと、
/// 継続プレイでは`dirty=false`だったはずの国家がロード後だけ余分に即時再評価
/// されてしまう(save/loadの決定性が壊れる)。そのため`dirty`を実際の値のまま
/// 保存・復元する。`#[serde(default = "default_dirty_true")]`により、この
/// フィールドを持たない旧SaveはP21-SAVE-002A1と同じ安全側(`true`=初回即時再評価)
/// から復元される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedCountryAiState {
    pub country_id: CountryId,
    pub mode: CountryAiMode,
    pub decision_reason: CountryAiDecisionReason,
    pub last_daily_evaluation_day: u32,
    pub last_weekly_evaluation_day: u32,
    pub last_monthly_evaluation_day: u32,
    pub cooldown_until_day: u32,
    #[serde(default = "default_dirty_true")]
    pub dirty: bool,
    /// P26-008-A: 募兵AI補給ゲートの現在状態。`#[serde(default)]`(=`false`)により、
    /// このフィールドを持たない旧Saveは「未停止」から復元される――次回の正規月次
    /// 募兵評価で、その時点の実際の備蓄・需要・配送状態から改めて判定される
    /// (ロード直後に月次cadenceを無視した臨時判定は行わない、spec §18)。
    #[serde(default)]
    pub recruitment_supply_blocked: bool,
}

/// `war::military_ai::MilitaryAiState`の正規状態DTO。`dirty`往復の理由は
/// `SavedCountryAiState`と同じ(P26-008-PRE-002、上記doc参照)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedMilitaryAiState {
    pub country_id: CountryId,
    pub last_evaluated_day: u32,
    pub last_decision_reason: MilitaryAiDecisionReason,
    pub estimated_own_power: u64,
    pub estimated_enemy_power: u64,
    #[serde(default = "default_dirty_true")]
    pub dirty: bool,
}

/// `SavedCountryAiState::dirty`/`SavedMilitaryAiState::dirty`の`#[serde(default)]`用。
/// このフィールドを持たない旧Saveは、P21-SAVE-002A1が元々定めた安全側の初期値
/// (`true`=ロード後最初の評価機会で必ず1回再評価する)から復元される。
fn default_dirty_true() -> bool {
    true
}

/// `country::country_ai::CountryAiRegistry`のコンテナDTO。
///
/// レジストリ直下の`dirty: bool`は保存対象から除外した。P21-SAVE-002A1で実コードを
/// 全文検索した結果、`CountryAiRegistry.dirty`への書き込み箇所(`mark_all_dirty`/
/// `mark_country_dirty`)は存在するが、読み取り箇所が一切存在しない(常に国ごとの
/// `CountryAiState.dirty`だけが`is_weekly_due`/`is_monthly_due`判定で参照される)ため、
/// 実質的に未使用のブックキーピング状態であり、正規状態でも派生の再評価トリガーとしても
/// 機能していないと判断した。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedCountryAiRegistry {
    pub ai_states: HashMap<CountryId, SavedCountryAiState>,
    /// P29-005: AI 募兵の能力推定に使う月次産出の履歴。追加フィールド(旧 Save には存在せず、
    /// `#[serde(default)]`により空として復元する。過去の月を推測して埋めない)。
    #[serde(default)]
    pub supply_production_history:
        HashMap<CountryId, crate::military::supply_production_history::SupplyProductionHistory>,
    /// P29-009: 建設 AI の Iron 不足認識用の履歴。追加フィールド(旧 Save では空 = 未集計)。
    #[serde(default)]
    pub iron_shortage_history:
        HashMap<CountryId, crate::economy::iron_shortage::IronShortageHistory>,
}

/// `war::military_ai::MilitaryAiRegistry`のコンテナDTO。理由は`SavedCountryAiRegistry`と
/// 同じ(P21-SAVE-002A1で確認: レジストリ直下の`MilitaryAiRegistry.dirty`も書き込み箇所
/// [`mark_all_dirty`/`mark_country_dirty`]のみで読み取り箇所が実コード中に存在しない)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedMilitaryAiRegistry {
    pub ai_states: HashMap<CountryId, SavedMilitaryAiState>,
}

/// `military::data::MilitaryRegistry`の可変部分(師団)のみのDTO。
/// `definitions`(師団定義、静的マスターデータ)は含めない。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedMilitaryRegistry {
    pub divisions: HashMap<DivisionId, Division>,
    pub next_division_id: usize,
    /// P26-004: Divisionごとの日次補給充足率(`military::data::MilitaryRegistry::
    /// division_supply_fulfillment`)。旧Save(このフィールド自体が存在しない)は
    /// `#[serde(default)]`により空`HashMap`として読み込まれ、`supply_fulfillment_of`が
    /// 欠落キーを`1.0`として扱うため、旧Saveのロード直後は全Divisionが暗黙に
    /// 完全充足(1.0)として振る舞う(spec §8のdefault要件そのもの)。
    #[serde(default)]
    pub division_supply_fulfillment: HashMap<DivisionId, f64>,
}

/// `military::battle::BattleRegistry`のコンテナDTO。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedBattleRegistry {
    pub battles: HashMap<BattleId, Battle>,
    pub next_id: usize,
}

/// `military::army::ArmyRegistry`のコンテナDTO。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedArmyRegistry {
    pub armies: HashMap<ArmyId, Army>,
    /// DivisionId -> ArmyId のマッピング(1師団は1編成のみ所属)
    pub division_army_map: HashMap<DivisionId, ArmyId>,
    pub next_id: usize,
    /// 国家ごとの自動採番カウンタ("Army 1", "Army 2", ...)
    pub next_army_number: HashMap<CountryId, u32>,
}

/// `war::frontline::FrontlineRegistry`のコンテナDTO。
///
/// P21-SAVE-002A決定: ArmyIdとの新しい接続・攻勢線・作戦命令などは追加せず、
/// 現在実在する`FrontlineRegistry`の正規データ(当時5フィールド)だけを表現する方針だった。
/// P21-005で`army_frontline_map`(Army↔Frontline割当の正規情報)を追加する。
/// `FrontlinePlan.assigned_army_ids`は`FrontlinePlan`型自体を再利用するため
/// (`#[serde(default)]`付き)、このDTOへの追加フィールドは不要。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedFrontlineRegistry {
    pub frontlines: HashMap<FrontlineId, Frontline>,
    /// key: (FrontlineId, CountryId)
    pub plans: HashMap<(FrontlineId, CountryId), FrontlinePlan>,
    pub next_frontline_id: usize,
    pub division_frontline_map: HashMap<DivisionId, FrontlineId>,
    /// 前線によって自動生成された移動命令を実行中の陸軍セット
    pub frontline_generated_movements: HashSet<DivisionId>,
    /// P21-005: ArmyId -> FrontlineId のマッピング。旧セーブに存在しない場合は空として読み込む。
    #[serde(default)]
    pub army_frontline_map: HashMap<ArmyId, FrontlineId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, StateId};
    use crate::country::{EconomicSystem, GovernmentType};
    use crate::diplomacy::claims::ClaimSource;
    use crate::diplomacy::crisis::CrisisPhase;
    use crate::military::data::{DivisionSize, DivisionStatus, DivisionType};
    use crate::war::frontline::FrontlineStance;

    /// 全フィールドを埋めた代表的な`SaveGameV1`を構築する(往復テスト共通のfixture)。
    /// 各種`next_*`発行値・移動途中Division・Army所属関係・Claim/Crisisを含む、
    /// 「実際に意味のある状態」を持たせる(空のデフォルト値だけでは往復の意味がないため)。
    fn representative_save() -> SaveGameV1 {
        let division = Division {
            id: DivisionId(7),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(3),
            destination: Some(StateId(9)),
            current_path: vec![StateId(4), StateId(9)],
            target_state: None,
            manpower: 8_500,
            max_manpower: 10_000,
            equipment: 90.0,
            max_equipment: 100.0,
            organization: 42.5,
            max_organization: 100.0,
            morale: 0.8,
            max_morale: 1.0,
            experience: 3.5,
            supply_ratio: 0.95,
            movement_progress: 0.42,
            status: DivisionStatus::Moving,
            def_id: DivisionDefinitionId(1),
            attack_power: 12,
            defense_power: 8,
            combat_id: None,
        };

        let mut divisions = HashMap::new();
        divisions.insert(division.id, division);

        let army = Army {
            id: ArmyId(2),
            owner: CountryId(1),
            name: "Army 1".to_string(),
            member_division_ids: vec![DivisionId(7)],
        };
        let mut armies = HashMap::new();
        armies.insert(army.id, army);
        let mut division_army_map = HashMap::new();
        division_army_map.insert(DivisionId(7), ArmyId(2));
        let mut next_army_number = HashMap::new();
        next_army_number.insert(CountryId(1), 2u32);

        let claim = TerritorialClaim {
            id: ClaimId(0),
            claimant_country: CountryId(1),
            target_state: StateId(9),
            strength: 42.0,
            created_date: "1800/03/01".to_string(),
            is_permanent: false,
            status: crate::diplomacy::claims::ClaimStatus::Active,
            source: ClaimSource::BorderDispute,
        };
        let mut claims = HashMap::new();
        claims.insert(claim.id, claim);

        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(1),
            target: CountryId(2),
            war_goals: Vec::new(),
            start_date: "1800/02/01".to_string(),
            current_phase: CrisisPhase::Negotiating,
            escalation: 15.0,
            initiator_support: 60.0,
            target_resistance: 40.0,
            days_in_phase: 3,
            deadline_date: Some("1800/03/01".to_string()),
            international_concern: 5.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        };
        let mut crises = HashMap::new();
        crises.insert(crisis.id, crisis);

        let country = CountryData {
            id: CountryId(1),
            name: "Test Country".to_string(),
            map_color: [0.2, 0.4, 0.6],
            capital_state_id: StateId(0),
            treasury: 5_000.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::Mercantilism,
            ..CountryData::default()
        };

        let state = StateData {
            id: StateId(9),
            name: "Contested State".to_string(),
            owner_country_id: CountryId(2),
            controller_country: Some(CountryId(1)),
            occupation_progress: 55.0,
            ..StateData::default()
        };

        let mut country_ai_states = HashMap::new();
        country_ai_states.insert(
            CountryId(2),
            SavedCountryAiState {
                country_id: CountryId(2),
                mode: CountryAiMode::AtWar,
                decision_reason: CountryAiDecisionReason::WarInProgress,
                last_daily_evaluation_day: 10,
                last_weekly_evaluation_day: 7,
                last_monthly_evaluation_day: 0,
                cooldown_until_day: 15,
                dirty: false,
                recruitment_supply_blocked: false,
            },
        );

        let mut military_ai_states = HashMap::new();
        military_ai_states.insert(
            CountryId(2),
            SavedMilitaryAiState {
                country_id: CountryId(2),
                last_evaluated_day: 10,
                last_decision_reason: MilitaryAiDecisionReason::SufficientAdvantage,
                estimated_own_power: 5_000,
                estimated_enemy_power: 3_000,
                dirty: false,
            },
        );

        SaveGameV1 {
            version: SAVE_FORMAT_VERSION_V1,
            date: SavedGameDate {
                year: 1801,
                month: 6,
                day: 15,
                accumulator: 0.37,
            },
            game_speed: 2,
            player_country: Some(CountryId(1)),
            world_civilization: SavedWorldCivilizationState {
                current_stage: WorldStage::IndustrialRevolution,
                milestone_countries: HashMap::new(),
                last_advanced_date: "1801/01/01".to_string(),
            },
            countries: vec![country],
            states: vec![state],
            diplomacy: SavedDiplomacyRegistry::default(),
            war_justifications: SavedWarJustificationRegistry::default(),
            wars: SavedWarRegistry::default(),
            claims: SavedClaimRegistry { claims, next_id: 1 },
            crises: SavedCrisisRegistry { crises, next_id: 1 },
            country_ai: SavedCountryAiRegistry {
                ai_states: country_ai_states,
                supply_production_history: Default::default(),
                iron_shortage_history: Default::default(),
            },
            military_ai: SavedMilitaryAiRegistry {
                ai_states: military_ai_states,
            },
            military: SavedMilitaryRegistry {
                divisions,
                next_division_id: 8,
                division_supply_fulfillment: [(DivisionId(7), 0.75_f64)].into_iter().collect(),
            },
            battles: SavedBattleRegistry::default(),
            armies: SavedArmyRegistry {
                armies,
                division_army_map,
                next_id: 3,
                next_army_number,
            },
            frontlines: SavedFrontlineRegistry::default(),
        }
    }

    /// RONへSerializeし、Deserializeで復元したものを返す(往復テスト共通ヘルパー)。
    fn round_trip(save: &SaveGameV1) -> SaveGameV1 {
        let ron_str = ron::to_string(save).expect("SaveGameV1 must serialize to RON");
        ron::from_str(&ron_str).expect("serialized SaveGameV1 RON must deserialize back")
    }

    #[test]
    fn representative_save_serializes_to_ron() {
        let save = representative_save();
        let ron_str = ron::to_string(&save).expect("SaveGameV1 must serialize to RON");
        assert!(!ron_str.is_empty());
    }

    #[test]
    fn round_trip_preserves_world_and_progress_fields() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(restored.version, original.version);
        assert_eq!(restored.date, original.date);
        assert_eq!(restored.game_speed, original.game_speed);
        assert_eq!(restored.player_country, original.player_country);
        assert_eq!(restored.world_civilization, original.world_civilization);
    }

    #[test]
    fn round_trip_preserves_country_and_state_representative_fields() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(restored.countries.len(), original.countries.len());
        let orig_country = &original.countries[0];
        let rest_country = &restored.countries[0];
        assert_eq!(rest_country.id, orig_country.id);
        assert_eq!(rest_country.name, orig_country.name);
        assert_eq!(rest_country.map_color, orig_country.map_color);
        assert_eq!(rest_country.treasury, orig_country.treasury);
        assert_eq!(
            rest_country.available_manpower,
            orig_country.available_manpower
        );

        assert_eq!(restored.states.len(), original.states.len());
        let orig_state = &original.states[0];
        let rest_state = &restored.states[0];
        assert_eq!(rest_state.id, orig_state.id);
        assert_eq!(rest_state.owner_country_id, orig_state.owner_country_id);
        assert_eq!(rest_state.controller_country, orig_state.controller_country);
        assert_eq!(
            rest_state.occupation_progress,
            orig_state.occupation_progress
        );
    }

    /// 要求テスト項目: RONへversionフィールドが含まれる。
    #[test]
    fn serialized_ron_contains_version_field() {
        let save = representative_save();
        let ron_str = ron::to_string(&save).expect("SaveGameV1 must serialize to RON");
        assert!(
            ron_str.contains("version"),
            "serialized RON must contain the version field: {ron_str}"
        );
    }

    /// 要求テスト項目: versionフィールドを欠いたRONはDeserializeに失敗する
    /// (`#[serde(default)]`を付けていないため、暗黙にV1として解釈されない)。
    #[test]
    fn deserialize_fails_when_version_field_is_missing() {
        let save = representative_save();
        let ron_str = ron::to_string(&save).expect("SaveGameV1 must serialize to RON");

        // compactなron::to_stringはフィールド宣言順に "version:1," のように出力する
        // (PrettyConfigを使っていないため改行・空白を含まない)。version エントリだけを
        // 除去することで、他のフィールドは正常なまま version だけを欠落させる。
        let without_version = ron_str.replacen("version:1,", "", 1);
        assert_ne!(
            without_version, ron_str,
            "test setup must actually remove the version field for this test to be meaningful"
        );

        let result: Result<SaveGameV1, _> = ron::from_str(&without_version);
        assert!(
            result.is_err(),
            "deserializing RON without a version field must fail, not silently default to V1"
        );
    }

    /// 要求テスト項目: Claim DTOが往復で失われない。
    #[test]
    fn round_trip_preserves_claim_registry() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(restored.claims.next_id, original.claims.next_id);
        assert_eq!(restored.claims.claims.len(), original.claims.claims.len());
        let orig_claim = original.claims.claims.get(&ClaimId(0)).unwrap();
        let rest_claim = restored.claims.claims.get(&ClaimId(0)).unwrap();
        assert_eq!(rest_claim.claimant_country, orig_claim.claimant_country);
        assert_eq!(rest_claim.target_state, orig_claim.target_state);
        assert_eq!(rest_claim.strength, orig_claim.strength);
        assert_eq!(rest_claim.source, orig_claim.source);
    }

    /// 要求テスト項目: Crisis DTOが往復で失われない。
    #[test]
    fn round_trip_preserves_crisis_registry() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(restored.crises.next_id, original.crises.next_id);
        assert_eq!(restored.crises.crises.len(), original.crises.crises.len());
        let orig_crisis = original.crises.crises.get(&DiplomaticCrisisId(0)).unwrap();
        let rest_crisis = restored.crises.crises.get(&DiplomaticCrisisId(0)).unwrap();
        assert_eq!(rest_crisis.initiator, orig_crisis.initiator);
        assert_eq!(rest_crisis.target, orig_crisis.target);
        assert_eq!(rest_crisis.current_phase, orig_crisis.current_phase);
        assert_eq!(rest_crisis.escalation, orig_crisis.escalation);
    }

    /// 要求テスト項目: 各動的IDの次回発行値が往復で維持される
    /// (DivisionId/ArmyId/WarId/BattleId/ClaimId/DiplomaticCrisisId/FrontlineId)。
    #[test]
    fn round_trip_preserves_all_next_id_counters() {
        let mut original = representative_save();
        // wars/battles/frontlinesはfixtureでデフォルト(0)のままだと「維持された」ことの
        // 検証にならないため、代表値を明示的に設定してから往復させる。
        original.wars.next_id = 4;
        original.battles.next_id = 6;
        original.frontlines.next_frontline_id = 2;

        let restored = round_trip(&original);

        assert_eq!(
            restored.military.next_division_id,
            original.military.next_division_id
        );
        assert_eq!(restored.armies.next_id, original.armies.next_id);
        assert_eq!(restored.wars.next_id, original.wars.next_id);
        assert_eq!(restored.battles.next_id, original.battles.next_id);
        assert_eq!(restored.claims.next_id, original.claims.next_id);
        assert_eq!(restored.crises.next_id, original.crises.next_id);
        assert_eq!(
            restored.frontlines.next_frontline_id,
            original.frontlines.next_frontline_id
        );
    }

    /// 要求テスト項目: 移動途中Divisionの全移動フィールドがRON往復で維持される。
    #[test]
    fn round_trip_preserves_division_in_progress_movement_fields() {
        let original = representative_save();
        let restored = round_trip(&original);

        let orig_division = original.military.divisions.get(&DivisionId(7)).unwrap();
        let rest_division = restored.military.divisions.get(&DivisionId(7)).unwrap();

        assert_eq!(rest_division.status, orig_division.status);
        assert_eq!(rest_division.destination, orig_division.destination);
        assert_eq!(rest_division.current_path, orig_division.current_path);
        assert_eq!(rest_division.target_state, orig_division.target_state);
        assert_eq!(
            rest_division.movement_progress,
            orig_division.movement_progress
        );
        assert_eq!(orig_division.status, DivisionStatus::Moving);
        assert!(orig_division.destination.is_some());
        assert!(!orig_division.current_path.is_empty());
    }

    /// P26-004: `division_supply_fulfillment`(Division日次補給充足率)がRON往復で維持される。
    #[test]
    fn round_trip_preserves_division_supply_fulfillment() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(
            restored.military.division_supply_fulfillment,
            original.military.division_supply_fulfillment
        );
        assert_eq!(
            restored.military.division_supply_fulfillment.get(&DivisionId(7)),
            Some(&0.75)
        );
    }

    /// P26-004: `division_supply_fulfillment`フィールド自体が存在しない旧Saveは
    /// `#[serde(default)]`により空`HashMap`として読み込まれる(Save V1構文互換性)。
    #[test]
    fn old_save_ron_without_division_supply_fulfillment_field_loads_as_empty_map() {
        let ron = r#"(
            divisions: {},
            next_division_id: 0,
        )"#;
        let military: SavedMilitaryRegistry =
            ron::from_str(ron).expect("minimal SavedMilitaryRegistry RON must parse");
        assert!(military.division_supply_fulfillment.is_empty());
    }

    /// 要求テスト項目: Army所属関係がRON往復で維持される。
    #[test]
    fn round_trip_preserves_army_membership() {
        let original = representative_save();
        let restored = round_trip(&original);

        assert_eq!(
            restored.armies.division_army_map.get(&DivisionId(7)),
            original.armies.division_army_map.get(&DivisionId(7))
        );
        let orig_army = original.armies.armies.get(&ArmyId(2)).unwrap();
        let rest_army = restored.armies.armies.get(&ArmyId(2)).unwrap();
        assert_eq!(rest_army.member_division_ids, orig_army.member_division_ids);
        assert_eq!(rest_army.owner, orig_army.owner);
        assert_eq!(rest_army.name, orig_army.name);
        assert_eq!(
            restored.armies.next_army_number.get(&CountryId(1)),
            original.armies.next_army_number.get(&CountryId(1))
        );
    }

    /// 要求テスト項目: FrontlineRegistryの正規データ(前線・作戦命令・前線生成移動)が
    /// RON往復で維持される(タプルキーのHashMap<(FrontlineId, CountryId), _>を含む)。
    #[test]
    fn round_trip_preserves_frontline_registry_including_tuple_keyed_plans() {
        let mut original = representative_save();

        let frontline = Frontline {
            frontline_id: FrontlineId(0),
            war_id: WarId(0),
            attacker_country_id: CountryId(1),
            defender_country_id: CountryId(2),
            attacker_front_regions: vec![StateId(3)],
            defender_front_regions: vec![StateId(4)],
            border_region_pairs: vec![(StateId(3), StateId(4))],
        };
        original
            .frontlines
            .frontlines
            .insert(frontline.frontline_id, frontline);

        let plan = FrontlinePlan {
            frontline_id: FrontlineId(0),
            commanding_country_id: CountryId(1),
            stance: FrontlineStance::Offensive,
            objective_region_id: Some(StateId(4)),
            assigned_division_ids: vec![DivisionId(7)],
            assigned_army_ids: vec![ArmyId(2)],
            offensive_line_region_ids: Some(vec![StateId(3)]),
            offensive_supply_blocked: false,
        };
        original
            .frontlines
            .plans
            .insert((FrontlineId(0), CountryId(1)), plan);
        original
            .frontlines
            .division_frontline_map
            .insert(DivisionId(7), FrontlineId(0));
        original
            .frontlines
            .frontline_generated_movements
            .insert(DivisionId(7));
        original
            .frontlines
            .army_frontline_map
            .insert(ArmyId(2), FrontlineId(0));
        original.frontlines.next_frontline_id = 1;

        let restored = round_trip(&original);

        let rest_plan = restored
            .frontlines
            .plans
            .get(&(FrontlineId(0), CountryId(1)))
            .expect("tuple-keyed plan must round-trip through RON");
        assert_eq!(rest_plan.stance, FrontlineStance::Offensive);
        assert_eq!(rest_plan.assigned_division_ids, vec![DivisionId(7)]);
        assert_eq!(rest_plan.assigned_army_ids, vec![ArmyId(2)]);
        assert_eq!(
            rest_plan.offensive_line_region_ids,
            Some(vec![StateId(3)]),
            "P21-007: offensive_line_region_ids must round-trip through RON"
        );
        assert_eq!(
            restored
                .frontlines
                .division_frontline_map
                .get(&DivisionId(7)),
            Some(&FrontlineId(0))
        );
        assert!(
            restored
                .frontlines
                .frontline_generated_movements
                .contains(&DivisionId(7))
        );
        assert_eq!(
            restored.frontlines.army_frontline_map.get(&ArmyId(2)),
            Some(&FrontlineId(0)),
            "P21-005: army_frontline_map must round-trip through RON"
        );
    }

    /// P21-005要求テスト項目47: 旧形式(assigned_army_ids/army_frontline_mapフィールド自体が
    /// 存在しないRON)を読み込んだ場合、Army前線割当は空として復元される
    /// (`#[serde(default)]`によるフィールド欠落耐性)。`deserialize_fails_when_version_field_is_missing`
    /// と同じ手法(実際のシリアライズ結果から対象フィールドだけを文字列除去する)を使う。
    #[test]
    fn old_format_ron_without_army_frontline_fields_loads_as_empty() {
        let plan = FrontlinePlan {
            frontline_id: FrontlineId(0),
            commanding_country_id: CountryId(1),
            stance: FrontlineStance::Offensive,
            objective_region_id: Some(StateId(4)),
            assigned_division_ids: vec![DivisionId(7)],
            assigned_army_ids: vec![ArmyId(2)],
            offensive_line_region_ids: None,
            offensive_supply_blocked: false,
        };
        let plan_ron = ron::to_string(&plan).unwrap();
        assert!(plan_ron.contains("assigned_army_ids"));
        let without_field = plan_ron.replacen(",assigned_army_ids:[(2)]", "", 1);
        assert_ne!(
            without_field, plan_ron,
            "test setup must actually remove the assigned_army_ids field"
        );
        let restored: FrontlinePlan = ron::from_str(&without_field).expect(
            "FrontlinePlan RON missing assigned_army_ids must still deserialize (serde default)",
        );
        assert!(restored.assigned_army_ids.is_empty());
        assert_eq!(restored.assigned_division_ids, vec![DivisionId(7)]);

        // SavedFrontlineRegistry全体でもarmy_frontline_mapフィールド自体の欠落に耐える。
        let registry = SavedFrontlineRegistry {
            next_frontline_id: 1,
            ..Default::default()
        };
        let registry_ron = ron::to_string(&registry).unwrap();
        assert!(registry_ron.contains("army_frontline_map"));
        let without_map_field = registry_ron.replacen(",army_frontline_map:{}", "", 1);
        assert_ne!(
            without_map_field, registry_ron,
            "test setup must actually remove the army_frontline_map field"
        );
        let restored_registry: SavedFrontlineRegistry = ron::from_str(&without_map_field)
            .expect("SavedFrontlineRegistry RON missing army_frontline_map must still deserialize");
        assert!(restored_registry.army_frontline_map.is_empty());
    }

    /// 要求テスト項目14: 旧形式(offensive_line_region_idsフィールド自体が存在しないRON、
    /// P21-007以前のセーブ相当)を読み込んだ場合、攻勢線は未設定(None)として復元される
    /// (`#[serde(default)]`によるフィールド欠落耐性)。
    #[test]
    fn old_format_ron_without_offensive_line_field_loads_as_none() {
        let plan = FrontlinePlan {
            frontline_id: FrontlineId(0),
            commanding_country_id: CountryId(1),
            stance: FrontlineStance::Offensive,
            objective_region_id: Some(StateId(4)),
            assigned_division_ids: vec![DivisionId(7)],
            assigned_army_ids: vec![ArmyId(2)],
            offensive_line_region_ids: Some(vec![StateId(3)]),
            offensive_supply_blocked: false,
        };
        let plan_ron = ron::to_string(&plan).unwrap();
        assert!(plan_ron.contains("offensive_line_region_ids"));
        let without_field = plan_ron.replacen(",offensive_line_region_ids:Some([(3)])", "", 1);
        assert_ne!(
            without_field, plan_ron,
            "test setup must actually remove the offensive_line_region_ids field"
        );
        let restored: FrontlinePlan = ron::from_str(&without_field).expect(
            "FrontlinePlan RON missing offensive_line_region_ids must still deserialize (serde default)",
        );
        assert_eq!(restored.offensive_line_region_ids, None);
        assert_eq!(restored.assigned_division_ids, vec![DivisionId(7)]);
    }

    /// 要求テスト項目: SaveGameV1/補助DTOがEntityを要求しない。
    ///
    /// このファイル(`dto.rs`)は`bevy`を一度もimportしていない(ファイル冒頭のuse文一覧を
    /// 参照)。`bevy::prelude::Entity`型を経由する余地が構造的に存在しないことを、
    /// 代表的な値を実際に構築・シリアライズすることで併せて確認する。
    #[test]
    fn save_game_v1_never_references_bevy_entity() {
        let save = representative_save();
        // 構築できてSerializeが通る時点で、Entity型のフィールドが存在しないことは
        // コンパイル時に保証されている(このファイルはbevy cratesを一切importしていない)。
        ron::to_string(&save).expect("SaveGameV1 must serialize without any Entity dependency");
    }

    /// 要求テスト項目: paused・カメラ・UI選択状態をDTOへ含めていない。
    ///
    /// `SaveGameV1`のフィールドはこのファイル上部の構造体定義で全て確認できる
    /// (`GamePaused`/カメラ`Transform`/`CameraDragState`/`SelectedDivision`/`SelectedArmy`/
    /// `SelectedState`/各種`*PanelState`に対応するフィールドは一つも存在しない)。
    /// この関数はその事実を、実際に全フィールドを埋めた値を構築することで裏付ける
    /// (存在しないフィールドを設定しようとすればコンパイルエラーになるため、
    /// 構造体リテラルが書けること自体が「含まれていない」ことの構造的な証拠になる)。
    #[test]
    fn save_game_v1_excludes_paused_camera_and_ui_selection_state() {
        let _save = representative_save();
    }

    /// 要求テスト項目1: AIの正規状態がRON往復で維持される。
    /// `SavedCountryAiState`/`SavedMilitaryAiState`が`PartialEq`を実装しているため、
    /// 個別フィールドではなく値全体の`==`で比較できる。
    #[test]
    fn round_trip_preserves_ai_normative_state() {
        let original = representative_save();
        let restored = round_trip(&original);

        let orig_country_ai = original.country_ai.ai_states.get(&CountryId(2)).unwrap();
        let rest_country_ai = restored.country_ai.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(rest_country_ai, orig_country_ai);
        assert_eq!(orig_country_ai.mode, CountryAiMode::AtWar);
        assert_eq!(orig_country_ai.cooldown_until_day, 15);

        let orig_military_ai = original.military_ai.ai_states.get(&CountryId(2)).unwrap();
        let rest_military_ai = restored.military_ai.ai_states.get(&CountryId(2)).unwrap();
        assert_eq!(rest_military_ai, orig_military_ai);
        assert_eq!(orig_military_ai.estimated_own_power, 5_000);
    }

    /// P26-008-PRE-002: `SavedCountryAiState`/`SavedMilitaryAiState`の`dirty`は
    /// もはや除外される派生キャッシュではなく、正規状態として往復する
    /// (`war::ai_change_tracking`によるイベント駆動dirty化のもとでは、ロード後の
    /// 次回評価タイミングが継続プレイと一致するために必要、`save/dto.rs`冒頭doc参照)。
    /// `true`/`false`の両方を明示したリテラルで、RONへ実際に出力され、値が
    /// 往復で保たれることを確認する(全フィールドを名前で列挙し、`..Default`等の
    /// ショートカットを使わない――新フィールド追加時もこのリテラルの網羅性を保つ)。
    #[test]
    fn saved_ai_state_dtos_hold_a_meaningful_dirty_field_that_round_trips() {
        let country_state_dirty = SavedCountryAiState {
            country_id: CountryId(2),
            mode: CountryAiMode::Developing,
            decision_reason: CountryAiDecisionReason::PeacetimeDevelopment,
            last_daily_evaluation_day: 0,
            last_weekly_evaluation_day: 0,
            last_monthly_evaluation_day: 0,
            cooldown_until_day: 0,
            dirty: true,
            recruitment_supply_blocked: false,
        };
        let country_state_settled = SavedCountryAiState { dirty: false, ..country_state_dirty.clone() };
        let military_state_dirty = SavedMilitaryAiState {
            country_id: CountryId(2),
            last_evaluated_day: 0,
            last_decision_reason: MilitaryAiDecisionReason::NoActiveWar,
            estimated_own_power: 0,
            estimated_enemy_power: 0,
            dirty: true,
        };
        let military_state_settled = SavedMilitaryAiState { dirty: false, ..military_state_dirty.clone() };

        let ron_str = ron::to_string(&(
            &country_state_dirty,
            &country_state_settled,
            &military_state_dirty,
            &military_state_settled,
        ))
        .expect("AI state DTOs must serialize");
        assert!(
            ron_str.contains("dirty"),
            "AI state DTO RON must contain the now-meaningful 'dirty' field: {ron_str}"
        );

        let restored: (
            SavedCountryAiState,
            SavedCountryAiState,
            SavedMilitaryAiState,
            SavedMilitaryAiState,
        ) = ron::from_str(&ron_str).expect("must deserialize back");
        assert!(restored.0.dirty);
        assert!(!restored.1.dirty);
        assert!(restored.2.dirty);
        assert!(!restored.3.dirty);
    }

    /// 要求テスト項目2/3/5: `SavedCountryAiRegistry`/`SavedMilitaryAiRegistry`
    /// (国家単位ではなくレジストリ直下)には引き続き派生`dirty`フィールドが存在しない
    /// (P21-SAVE-002A1のまま、変更なし――未使用のブックキーピング状態であることは
    /// 今回のP26-008-PRE-002でも再確認済み、`export.rs`のdoc参照)。
    #[test]
    fn saved_ai_registries_have_no_derived_dirty_field() {
        // 型構造の確認: SavedCountryAiRegistry/SavedMilitaryAiRegistryはai_statesのみで
        // 構築できる(dirtyフィールドが存在すれば、このリテラルはコンパイルが通らない)。
        let empty_country_ai = SavedCountryAiRegistry {
            ai_states: HashMap::new(),
            supply_production_history: Default::default(),
            iron_shortage_history: Default::default(),
        };
        let empty_military_ai = SavedMilitaryAiRegistry {
            ai_states: HashMap::new(),
        };
        assert!(empty_country_ai.ai_states.is_empty());
        assert!(empty_military_ai.ai_states.is_empty());

        // 往復させても正規状態(国家単位のdirtyを含む)は失われない。
        let save = representative_save();
        let restored = round_trip(&save);
        assert_eq!(
            restored.country_ai.ai_states.len(),
            save.country_ai.ai_states.len()
        );
        assert_eq!(
            restored.military_ai.ai_states.len(),
            save.military_ai.ai_states.len()
        );
        assert_eq!(
            restored.country_ai.ai_states.get(&CountryId(2)).unwrap().dirty,
            save.country_ai.ai_states.get(&CountryId(2)).unwrap().dirty,
        );
        assert_eq!(
            restored.military_ai.ai_states.get(&CountryId(2)).unwrap().dirty,
            save.military_ai.ai_states.get(&CountryId(2)).unwrap().dirty,
        );
    }

    /// P29-005: AI 募兵の能力推定用の履歴は追加フィールド。旧 Save(フィールドなし)は空の履歴として
    /// 復元し、過去の月を推測して埋めない。新しい Save は履歴を往復で保持する。
    #[test]
    fn supply_production_history_is_additive_and_round_trips() {
        let old: SavedCountryAiRegistry =
            ron::from_str("(ai_states: {})").expect("old-format RON (no history field) must parse");
        assert!(old.supply_production_history.is_empty());

        let mut reg = SavedCountryAiRegistry::default();
        let mut h = crate::military::supply_production_history::SupplyProductionHistory::default();
        h.record(1800, 12, Some(100));
        h.record(1801, 1, None);
        h.record(1801, 2, Some(0));
        reg.supply_production_history.insert(CountryId(2), h.clone());
        let text = ron::to_string(&reg).unwrap();
        let back: SavedCountryAiRegistry = ron::from_str(&text).unwrap();
        assert_eq!(back.supply_production_history.get(&CountryId(2)), Some(&h));
    }

    /// P29-009: 建設 AI の Iron 不足の履歴は追加フィールド。旧 Save(フィールドなし)は空 = 未集計。
    #[test]
    fn iron_shortage_history_is_additive_and_round_trips() {
        let old: SavedCountryAiRegistry =
            ron::from_str("(ai_states: {})").expect("old-format RON (no history fields) must parse");
        assert!(old.iron_shortage_history.is_empty());

        let mut reg = SavedCountryAiRegistry::default();
        let mut h = crate::economy::iron_shortage::IronShortageHistory::default();
        h.record(1800, 12, Some(12.5), 3);
        h.record(1801, 1, None, 3);
        h.record(1801, 2, Some(0.0), 4);
        reg.iron_shortage_history.insert(CountryId(1), h.clone());
        let text = ron::to_string(&reg).unwrap();
        let back: SavedCountryAiRegistry = ron::from_str(&text).unwrap();
        assert_eq!(back.iron_shortage_history.get(&CountryId(1)), Some(&h));
    }
}
