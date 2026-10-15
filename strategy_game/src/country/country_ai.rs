use crate::app::time::DayChangedMessage;
use crate::building::construction::{ConstructionQueueItem, ConstructionStatus};
use crate::building::data::BuildingType;
use crate::common::{CountryId, StateId, WarId};
use crate::country::{CountryData, CountryRegistry, PlayerCountry};
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::crisis_response;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::economy::resources::ResourceType;
use crate::localization::{Loc, tf};
use crate::military::data::MilitaryRegistry;
use crate::military::pathfinding::find_path;
use crate::research::allocation::InProgressTech;
use crate::research::data::{TechnologyDefinition, TechnologyRegistry};
use crate::research::world_stage::WorldCivilizationState;
use crate::state::data::StateRegistry;
use crate::ui::notification::GameNotification;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::update_all_frontlines;
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, evaluate_division_power};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;

/// AI発の宣戦布告のうち、まだ通知(GameNotification)を発行していないもののキュー。
/// `process_war_declaration_ai`は純粋関数として保つため、直接通知を書かず
/// ここに積むだけにし、実際の翻訳・発行は`emit_ai_war_declaration_notifications`
/// システム(Bevyへのアクセスが必要)側で行う。
#[derive(Resource, Default)]
pub struct PendingAiWarDeclarations(pub Vec<(CountryId, CountryId, StateId, WarId)>);

/// 高位国家AIモード
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountryAiMode {
    #[default]
    Developing,
    Recovering,
    PreparingWar,
    AtWar,
    Disabled,
}

impl CountryAiMode {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            CountryAiMode::Developing => "country_ai_mode.developing",
            CountryAiMode::Recovering => "country_ai_mode.recovering",
            CountryAiMode::PreparingWar => "country_ai_mode.preparing_war",
            CountryAiMode::AtWar => "country_ai_mode.at_war",
            CountryAiMode::Disabled => "country_ai_mode.disabled",
        }
    }
}

/// 国家AIの判断理由
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CountryAiDecisionReason {
    #[default]
    PeacetimeDevelopment,
    FoodShortageFarmPriority,
    RawMaterialMinePriority,
    SteelShortageSteelMillPriority,
    FundsShortage,
    NoBuildableState,
    NoResearchableTech,
    PostWarCooldown,
    NoAvailableDivisions,
    NoReachableTargetCountry,
    InsufficientPowerAdvantage,
    TruceInEffect,
    JustificationInProgress,
    WarDeclarationPending,
    WarInProgress,
    PlayerControlled,
}

impl CountryAiDecisionReason {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    pub fn display_name(self) -> &'static str {
        match self {
            CountryAiDecisionReason::PeacetimeDevelopment => {
                "country_ai_reason.peacetime_development"
            }
            CountryAiDecisionReason::FoodShortageFarmPriority => {
                "country_ai_reason.food_shortage_farm_priority"
            }
            CountryAiDecisionReason::RawMaterialMinePriority => {
                "country_ai_reason.raw_material_mine_priority"
            }
            CountryAiDecisionReason::SteelShortageSteelMillPriority => {
                "country_ai_reason.steel_shortage_steel_mill_priority"
            }
            CountryAiDecisionReason::FundsShortage => "country_ai_reason.funds_shortage",
            CountryAiDecisionReason::NoBuildableState => "country_ai_reason.no_buildable_state",
            CountryAiDecisionReason::NoResearchableTech => "country_ai_reason.no_researchable_tech",
            CountryAiDecisionReason::PostWarCooldown => "country_ai_reason.post_war_cooldown",
            CountryAiDecisionReason::NoAvailableDivisions => {
                "country_ai_reason.no_available_divisions"
            }
            CountryAiDecisionReason::NoReachableTargetCountry => {
                "country_ai_reason.no_reachable_target_country"
            }
            CountryAiDecisionReason::InsufficientPowerAdvantage => {
                "country_ai_reason.insufficient_power_advantage"
            }
            CountryAiDecisionReason::TruceInEffect => "country_ai_reason.truce_in_effect",
            CountryAiDecisionReason::JustificationInProgress => {
                "country_ai_reason.justification_in_progress"
            }
            CountryAiDecisionReason::WarDeclarationPending => {
                "country_ai_reason.war_declaration_pending"
            }
            CountryAiDecisionReason::WarInProgress => "country_ai_reason.war_in_progress",
            CountryAiDecisionReason::PlayerControlled => "country_ai_reason.player_controlled",
        }
    }
}

/// 1国家ごとの国家AI状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountryAiState {
    pub country_id: CountryId,
    pub mode: CountryAiMode,
    pub decision_reason: CountryAiDecisionReason,
    pub last_daily_evaluation_day: u32,
    pub last_weekly_evaluation_day: u32,
    pub last_monthly_evaluation_day: u32,
    pub cooldown_until_day: u32,
    pub dirty: bool,
    /// P26-008-A: 募兵AI補給ゲートの現在状態(true=新規募兵を停止中)。
    /// `national_reserve_days`だけからは中間帯(停止未満・再開未満)の状態を
    /// 再導出できないため(真のヒステリシス、P26-008-PRE-001 §12の論理監査)、
    /// `cooldown_until_day`と同種の「世界状態から再導出できないAI決定メモリ」
    /// として保持する。既存の`decision_reason`(建設AI等、他サブシステムの
    /// 最新判断理由)やMilitary AI状態を意味の異なる代替ストレージとして
    /// 流用してはいない。
    pub recruitment_supply_blocked: bool,
}

impl CountryAiState {
    pub fn new(country_id: CountryId) -> Self {
        Self {
            country_id,
            mode: CountryAiMode::Developing,
            decision_reason: CountryAiDecisionReason::PeacetimeDevelopment,
            last_daily_evaluation_day: 0,
            last_weekly_evaluation_day: 0,
            last_monthly_evaluation_day: 0,
            cooldown_until_day: 0,
            dirty: true,
            recruitment_supply_blocked: false,
        }
    }
}

/// 全国家AI状態を管理するリソース
#[derive(Resource, Default, Debug, Clone, Serialize, Deserialize)]
pub struct CountryAiRegistry {
    pub ai_states: HashMap<CountryId, CountryAiState>,
    pub dirty: bool,
    /// P29-005: AI 募兵の能力推定に使う、軍用補給物資の実際の月次産出の履歴(国別)。
    /// Save V1 へは追加フィールドとして保存される(旧 Save では空)。
    #[serde(default)]
    pub supply_production_history:
        HashMap<CountryId, crate::military::supply_production_history::SupplyProductionHistory>,
    /// P29-009: 建設 AI の Iron 不足認識用の履歴(MilitaryFactory の Iron 未充足の要求量、国別)。
    /// Save V1 へは追加フィールドとして保存される(旧 Save では空 = 未集計)。
    #[serde(default)]
    pub iron_shortage_history:
        HashMap<CountryId, crate::economy::iron_shortage::IronShortageHistory>,
    /// P29-010: Factory 増設抑止(B2)用の履歴(国別)。Save V1 の追加フィールド(旧 Save では空)。
    #[serde(default)]
    pub factory_starvation_history:
        HashMap<CountryId, crate::economy::factory_starvation::FactoryStarvationHistory>,
}

impl CountryAiRegistry {
    pub fn get_or_create_mut(&mut self, country_id: CountryId) -> &mut CountryAiState {
        self.ai_states
            .entry(country_id)
            .or_insert_with(|| CountryAiState::new(country_id))
    }

    pub fn mark_all_dirty(&mut self) {
        for state in self.ai_states.values_mut() {
            state.dirty = true;
        }
        self.dirty = true;
    }

    pub fn mark_country_dirty(&mut self, country_id: CountryId) {
        if let Some(state) = self.ai_states.get_mut(&country_id) {
            state.dirty = true;
        }
        self.dirty = true;
    }
}

/// 国家の全生存有効陸戦力を計算
pub fn calculate_country_total_power(
    country_id: CountryId,
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> u64 {
    military_registry
        .divisions
        .values()
        .filter(|a| {
            a.owner == country_id
                && a.manpower > 0
                && a.status != crate::military::data::DivisionStatus::Destroyed
                && state_registry
                    .get(a.current_state)
                    .map(|s| !s.is_sea)
                    .unwrap_or(false)
        })
        .map(evaluate_division_power)
        .sum()
}

/// 全国家の総有効陸戦力を一括計算する (P20-008最適化: `process_war_preparation_ai`が
/// 候補国ごとに `calculate_country_total_power` を再計算していたO(countries)重複を排除する)。
/// `calculate_country_total_power` と全く同一のフィルタ条件・集計方法を用いるため、
/// 個別呼び出しの結果と完全に一致する(順序に依存しないu64合計のため計算順序の違いは影響しない)。
///
/// P20-008追補: `CountryId(pub usize)` が密な小さい整数キーであることを踏まえ、
/// `HashMap` ではなく `Vec` をインデックスとして使用する。8か国程度の小規模ワールドでは
/// `HashMap` のハッシュ計算・バケット確保という定数項オーバーヘッドが、削減できた
/// 走査コストを上回り性能回帰を引き起こしていたため(詳細は報告書のP20-008追補セクション参照)、
/// ハッシュ計算を伴わないVecインデックスへ置き換えることで小規模でもオーバーヘッドを抑える。
/// 値・優先順位・決定論への影響は無い(単純な合計・グルーピングの実装変更のみ)。
pub(super) fn compute_total_power_by_country(
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> Vec<u64> {
    let mut power_by_country: Vec<u64> = Vec::new();
    for division in military_registry.divisions.values() {
        if division.manpower == 0
            || division.status == crate::military::data::DivisionStatus::Destroyed
        {
            continue;
        }
        let is_land = state_registry
            .get(division.current_state)
            .map(|s| !s.is_sea)
            .unwrap_or(false);
        if !is_land {
            continue;
        }
        let idx = division.owner.0;
        if idx >= power_by_country.len() {
            power_by_country.resize(idx + 1, 0);
        }
        power_by_country[idx] += evaluate_division_power(division);
    }
    power_by_country
}

/// `compute_total_power_by_country` の結果からCountryIdの値を安全に引く
/// (登録済みでないIDは総戦力0として扱う。`calculate_country_total_power`が
/// 該当軍0件の国に対して0を返すのと同じ意味)。
fn total_power_for(power_by_country: &[u64], country_id: CountryId) -> u64 {
    power_by_country.get(country_id.0).copied().unwrap_or(0)
}

/// 全国家の実効支配陸上State一覧を一括計算する (P20-008最適化: `process_war_preparation_ai`が
/// 候補国ごとに `state_registry.states` 全件走査していたO(countries × states)重複を排除する)。
/// `state_registry.states` を1回だけ走査し、元の走査順序(StateId昇順)を保ったまま
/// 支配国ごとに振り分けるため、候補地域選択の優先順位(先頭要素優先)は変更しない。
///
/// P20-008追補: `compute_total_power_by_country` と同じ理由でVecインデックスを使用する。
fn compute_land_states_by_controller(state_registry: &StateRegistry) -> Vec<Vec<StateId>> {
    let mut states_by_controller: Vec<Vec<StateId>> = Vec::new();
    for state in &state_registry.states {
        if state.is_sea {
            continue;
        }
        let idx = state.controller().0;
        if idx >= states_by_controller.len() {
            states_by_controller.resize_with(idx + 1, Vec::new);
        }
        states_by_controller[idx].push(state.id);
    }
    states_by_controller
}

/// `compute_land_states_by_controller` の結果からCountryIdの値を安全に引く
/// (登録済みでないIDは陸上支配Stateなしとして扱う)。
fn land_states_for(states_by_controller: &[Vec<StateId>], country_id: CountryId) -> &[StateId] {
    states_by_controller
        .get(country_id.0)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

/// Mine建設候補州の探索結果 (P26-004-PRE-002)。
enum ConstructOutcome {
    /// 建設キューへ1件追加し国庫を減算した。
    Built,
    /// 建設可能な候補州が存在しない(資金は足りているが、対象が見つからない)。
    NoCandidate,
    /// 国庫が建設費に満たない。
    InsufficientFunds,
}

/// `target_building` の建設候補州を1件探す (StateId昇順で最初に条件を満たす州)。
/// Mineの場合、P26-004-PRE-002でクリスタル専用州の除外に加え、実際に採掘可能な
/// 鉱床(discoveredかつ非MagicCrystal、`process_country_production`のMine分岐
/// [`src/economy/production.rs`]と同一条件)が存在することを新たに要求する。
/// これにより、鉱床が一切ない州・国家へMineが選ばれることを防ぐ。
fn find_candidate_state(
    country: &CountryData,
    state_registry: &StateRegistry,
    owned_states: &[StateId],
    target_building: BuildingType,
    max_level: u32,
) -> Option<StateId> {
    owned_states.iter().copied().find(|&sid| {
        let in_queue = country
            .construction_queue
            .iter()
            .any(|item| item.state_id == sid && item.building_type == target_building);
        if in_queue {
            return false;
        }
        let Some(state_data) = state_registry.get(sid) else {
            return false;
        };
        if target_building == BuildingType::Mine {
            // P21-009-FIX-001: クリスタル専用州には通常Mineの対象鉱床が存在しないため、
            // AIも新規建設候補から除外する(CrystalMineはAIの建設判断対象外のまま)。
            if crate::economy::resources::is_crystal_only_state(&state_data.resource_deposits) {
                return false;
            }
            // P26-004-PRE-002: 実際に採掘可能な鉱床(discoveredかつ非MagicCrystal)が
            // 一切ない州は、鉱床皆無国家(Elfin/Oceanic等)がMineへ国庫を浪費しないよう
            // 候補から除外する。
            let has_usable_deposit = state_data
                .resource_deposits
                .iter()
                .any(|d| d.discovered && d.resource_type != ResourceType::MagicCrystal);
            if !has_usable_deposit {
                return false;
            }
        }
        state_data.building_level(target_building) < max_level
    })
}

/// `target_building` の建設を1件だけ試みる。資金不足・候補州なしのいずれの場合も
/// 国庫・建設キューを一切変更しない(原子性、P26-004-PRE-002 spec §6)。
fn try_construct(
    country: &mut CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    target_building: BuildingType,
) -> ConstructOutcome {
    let Some(def) = building_registry.get(target_building) else {
        return ConstructOutcome::NoCandidate;
    };
    if country.treasury < def.construction_cost {
        return ConstructOutcome::InsufficientFunds;
    }
    match find_candidate_state(country, state_registry, owned_states, target_building, def.max_level) {
        Some(state_id) => {
            let current_level = state_registry
                .get(state_id)
                .map(|s| s.building_level(target_building))
                .unwrap_or(0);
            country.treasury -= def.construction_cost;
            country.construction_queue.push(ConstructionQueueItem {
                state_id,
                building_type: target_building,
                target_level: current_level + 1,
                progress: 0.0,
                required_progress: def.required_progress,
                paid_cost: def.construction_cost,
                status: ConstructionStatus::InQueue,
            });
            ConstructOutcome::Built
        }
        None => ConstructOutcome::NoCandidate,
    }
}

/// P27-002-A: 優先度3(任意的な平時開発)が、建設費支払い後に国家の運転資金
/// (直近の月次経常支出の何か月分)を侵食しないよう要求する予備月数。優先度1
/// (Food緊急)・優先度2(P26-008-Cの供給網修復)には適用しない(spec §6)。
/// 正規定数は本モジュール内のここ1か所だけに定義する(spec §3)。
pub const PEACETIME_CONSTRUCTION_RESERVE_MONTHS: f64 = 3.0;

/// `try_construct`(優先度1・2が今も直接使う既存の共有関数、シグネチャ変更なし)を
/// 変更せず、優先度3専用のラッパーとして予備資金判定を追加する(spec §3・§6)。
///
/// 判定式(spec §3):
/// ```text
/// required_treasury = construction_cost + monthly_recurring_expenses × 3
/// allowed = treasury >= required_treasury
/// ```
///
/// `country.monthly_expenses`(`economy::finance::process_country_finance`が
/// 月次で確定させる既存の正規経常支出値、spec §4——建設費・軍事費・募兵費の
/// いずれも含まない)をそのまま再利用し、新しい式を複製しない。
///
/// 数値安全性(spec §5): `treasury`・`construction_cost`・`monthly_expenses`の
/// いずれかが非有限(NaN/Infinity)または負の場合、あるいは`required_treasury`
/// 自体の計算が非有限になる場合は、無効な財政値を0へ丸めて建設可能とはせず、
/// 安全側(`InsufficientFunds`、mutationなし)へ倒す。`def`が存在しない場合は
/// 財務判定を行わずそのまま`try_construct`へ委譲する(既存の`NoCandidate`挙動を
/// 変更しない)。
///
/// 予備判定に落ちた場合、`try_construct`自体は一切呼び出さない——候補探索・
/// 国庫・建設キューへの変更は発生しない(spec §7: 予備不足時に安価な代替候補を
/// 新たに探索しない、原子性を維持する)。
fn try_construct_peacetime(
    country: &mut CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    target_building: BuildingType,
) -> ConstructOutcome {
    if let Some(def) = building_registry.get(target_building) {
        let treasury = country.treasury;
        let construction_cost = def.construction_cost;
        let monthly_expenses = country.monthly_expenses;

        let values_valid = treasury.is_finite()
            && construction_cost.is_finite()
            && construction_cost >= 0.0
            && monthly_expenses.is_finite()
            && monthly_expenses >= 0.0;

        if !values_valid {
            return ConstructOutcome::InsufficientFunds;
        }

        let reserve = monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
        let required_treasury = construction_cost + reserve;

        if !required_treasury.is_finite() || treasury < required_treasury {
            return ConstructOutcome::InsufficientFunds;
        }
    }

    try_construct(country, state_registry, building_registry, owned_states, target_building)
}

/// 国家が現在保有する生産建物(level>0)が入力資源として要求する`ResourceType`集合を
/// 求める(`BuildingDefinition.input_resources`を汎用的に参照し、特定資源名を
/// 列挙しない、P26-004-PRE-002 spec §5.3)。HashMap/HashSetの反復順には最終結果が
/// 依存しない(呼び出し側が`ResourceType::ALL`の宣言順で走査するため)。
fn resource_types_required_by_owned_buildings(
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
) -> HashSet<ResourceType> {
    let mut needed = HashSet::new();
    for &sid in owned_states {
        let Some(state_data) = state_registry.get(sid) else {
            continue;
        };
        for (&b_type, &level) in state_data.buildings.iter() {
            if level == 0 {
                continue;
            }
            if let Some(def) = building_registry.get(b_type) {
                needed.extend(def.input_resources.keys().copied());
            }
        }
    }
    needed
}

/// 領内のいずれかの州で、指定`resource`のdiscovered鉱床とMineレベル>0が同居して
/// いるか(=その資源について既に稼働中の採掘経路を持つか)を判定する。
/// 既に稼働中なら、Dwarfのような既存供給網を持つ国家に事前開発規則を
/// 不必要に発火させない(P26-004-PRE-002 spec §5.3・Test E)。
fn has_active_mine_for_resource(
    state_registry: &StateRegistry,
    owned_states: &[StateId],
    resource: ResourceType,
) -> bool {
    owned_states.iter().any(|&sid| {
        state_registry.get(sid).is_some_and(|s| {
            s.building_level(BuildingType::Mine) > 0
                && s
                    .resource_deposits
                    .iter()
                    .any(|d| d.discovered && d.resource_type == resource)
        })
    })
}

/// 事前開発対象の未活用鉱床州を、指定`resource`について1件探す
/// (`find_candidate_state`のMine条件と同じ実採掘可能性・増設余地・重複キュー除外を
/// 満たす州のうち、StateId最小のもの)。
fn find_undeveloped_deposit_state(
    country: &CountryData,
    state_registry: &StateRegistry,
    owned_states: &[StateId],
    mine_max_level: u32,
    resource: ResourceType,
) -> Option<StateId> {
    owned_states.iter().copied().find(|&sid| {
        let in_queue = country.construction_queue.iter().any(|item| {
            item.state_id == sid && item.building_type == BuildingType::Mine
        });
        if in_queue {
            return false;
        }
        let Some(state_data) = state_registry.get(sid) else {
            return false;
        };
        if crate::economy::resources::is_crystal_only_state(&state_data.resource_deposits) {
            return false;
        }
        let has_resource_deposit = state_data
            .resource_deposits
            .iter()
            .any(|d| d.discovered && d.resource_type == resource);
        has_resource_deposit && state_data.building_level(BuildingType::Mine) < mine_max_level
    })
}

/// 領内の有効な州にFactory(level>0)が1つでも存在するか(=IndustrialGoods生産経路が
/// 構造的に存在するか、P26-008-C spec §7)。当月の実生産量(Coal入力不足・
/// workforce不足・財政破綻による稼働率低下等)とは無関係に判定する――既に存在する
/// Factoryを「稼働していないから存在しない」と誤認して重複建設しないための、
/// `has_active_mine_for_resource`と対になる構造的判定。
///
/// 完成済み(level>0)のFactoryに加え、**建設queue内に未完成のFactoryが1件でも
/// あれば**同様に「対応済み」とみなす(spec §7・§16.D: Factory建設中に同じ目的で
/// 別州へもう1件発注しない)。`find_candidate_state`自体は州単位の重複(同じ州へ
/// 同じ建物種別を二重に積む)しか排除しないため、複数州が候補になり得る場合の
/// 国家単位の重複はこの関数側で防ぐ。
fn has_active_factory_for_industrial_goods(
    country: &CountryData,
    state_registry: &StateRegistry,
    owned_states: &[StateId],
) -> bool {
    let already_built = owned_states
        .iter()
        .any(|&sid| state_registry.get(sid).is_some_and(|s| s.building_level(BuildingType::Factory) > 0));
    if already_built {
        return true;
    }
    country
        .construction_queue
        .iter()
        .any(|item| item.building_type == BuildingType::Factory)
}

/// P26-004-PRE-002が確立した「事前開発」対象を、鉱床由来の資源(Mine)だけでなく
/// IndustrialGoods(Factory)へ拡張したもの(P26-008-C)。
enum ProactiveDevelopmentTarget {
    Mine(StateId),
    Factory(StateId),
}

/// 保有する生産建物の入力資源のうち、対応する構造的生産経路がまだ存在しない資源に
/// ついて、事前開発すべき建物種別と建設候補州を1件だけ決定的に選ぶ。
/// 複数の資源が該当する場合は`ResourceType::ALL`の宣言順(Food,Wood,Iron,Coal,
/// MagicCrystal,IndustrialGoods,...)をtie-breakとして用いる(P26-004-PRE-002
/// spec §5.5、P26-008-C spec §9で維持)。
///
/// 鉱床由来の資源(Wood/Iron/Coal等)は既存どおりMine+未活用鉱床州で扱う。
/// `ResourceType::IndustrialGoods`だけは特別扱いし(鉱床を持たないため
/// `find_undeveloped_deposit_state`は使えない)、既存の汎用候補選択API
/// `find_candidate_state`をFactory向けにそのまま再利用する(P26-008-C spec §10:
/// 新しい生産グラフ探索を実装しない、既存APIの再利用に留める)。
/// MagicCrystalは対象外(CrystalRefinery系統の管轄)のまま。
fn find_proactive_development_target(
    country: &CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    mine_max_level: u32,
    factory_max_level: u32,
) -> Option<ProactiveDevelopmentTarget> {
    let needed = resource_types_required_by_owned_buildings(state_registry, building_registry, owned_states);
    if needed.is_empty() {
        return None;
    }
    for &resource in ResourceType::ALL.iter() {
        if resource == ResourceType::MagicCrystal || !needed.contains(&resource) {
            continue;
        }
        if resource == ResourceType::IndustrialGoods {
            if has_active_factory_for_industrial_goods(country, state_registry, owned_states) {
                continue;
            }
            if let Some(sid) = find_candidate_state(
                country,
                state_registry,
                owned_states,
                BuildingType::Factory,
                factory_max_level,
            ) {
                return Some(ProactiveDevelopmentTarget::Factory(sid));
            }
            continue;
        }
        if has_active_mine_for_resource(state_registry, owned_states, resource) {
            continue;
        }
        if let Some(sid) =
            find_undeveloped_deposit_state(country, state_registry, owned_states, mine_max_level, resource)
        {
            return Some(ProactiveDevelopmentTarget::Mine(sid));
        }
    }
    None
}

/// 財政AI(P27-002-C候補4、月次評価: game_day % 30 == country_id % 30 または dirty、
/// 建設AIより先に評価する)。
///
/// P27-002-B-PRE-001が確認した構造的財政赤字への対処として月次tax_rate自己調整を
/// 導入したが(P27-002-B)、P27-002-C-PRE-001の120か月監査により、単純な
/// 「黒字なら基準値0.15まで戻す」規則は長期的に不安定(Ferrowynの生活水準が
/// 0.00へ到達し税収が恒久的に消失、Arcadia/Dwarf/Oceanicも遅延して再崩壊)である
/// ことが判明した。本関数は、黒字時の引下げを「1段階下げても現在の経常支出を
/// 賄えると予測できる場合だけ」に限定する予測安全ゲート(`economy::finance::
/// decide_ai_tax_rate_delta`)へ置き換える。
///
/// 意思決定そのものは`decide_ai_tax_rate_delta`(純粋関数、当月の確定済み
/// `monthly_balance`・`monthly_expenses`と、当月の人口・人口加重平均生活水準だけを
/// 参照し、税収/行政費の式を複製しない)に委譲し、ここでは対象国家・州の取得と
/// 適用だけを行う。新しいAI状態・ヒステリシス・目標黒字額・将来予測シミュレーターは
/// 一切持たない(毎回既存の確定値だけから再導出するステートレスな規則)。
///
/// 既存の`tax_rate > 0.25`での生活水準低下ペナルティ(`economy::finance::
/// update_state_welfare_and_unrest`、無変更)は高税率のゲーム上の代償として
/// 引き続き機能する——本関数は税率を安全な範囲でしか下げないことでこのペナルティ
/// 圏内に長期間留まることを避けるが、ペナルティ自体を無効化するものではない。
pub fn process_tax_rate_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
) {
    let Some(country) = country_registry.get_mut(country_id) else {
        return;
    };

    // 人口加重平均生活水準(この加重平均を使う理由は`decide_ai_tax_rate_delta`の
    // doc参照: candidate_rate==tax_rateのとき予測値が実際のmonthly_incomeと
    // 数学的に完全一致するようにするため)。
    let mut total_population = 0u64;
    let mut weighted_living_standard = 0.0f64;
    for state in state_registry.states.iter().filter(|s| s.owner_country_id == country_id) {
        total_population += state.population;
        weighted_living_standard += state.population as f64 * state.living_standard as f64;
    }
    let average_living_standard = if total_population == 0 {
        0.0
    } else {
        (weighted_living_standard / total_population as f64) as f32
    };

    if let Some(delta) = crate::economy::finance::decide_ai_tax_rate_delta(
        country.monthly_balance,
        country.tax_rate,
        total_population,
        average_living_standard,
        country.monthly_expenses,
    ) {
        country.tax_rate = crate::economy::finance::apply_tax_rate_delta(country.tax_rate, delta);
    }
}

// ─── P28-002: 戦時MilitaryFactory建設 ────────────────────────────────────────
//
// `verification_logs/phase-28/p28-002-pre-001/`の候補C。戦時のAI国家で、
// MilitarySuppliesの実生産(前月の稼働率による月産)が現在の月次需要に届かない
// 場合、入力資源と資金が許す範囲でMilitaryFactoryを1件だけ建設する。攻勢補給
// ゲート・募兵ゲート・前線戦力比は一切変更しない——自国の生産で備蓄を回復させ、
// 既存ゲートの条件を正規に満たすための建設判断である。

/// 戦時MilitaryFactory建設で、建設費の支払い後も国庫に残すべき月次経常支出の
/// 月数。平時の任意開発(優先度3)の`PEACETIME_CONSTRUCTION_RESERVE_MONTHS`
/// (3か月)とは独立した定数で、平時建設には適用しない。3か月では交戦国が1件も
/// 建設できず、0か月では国庫がほぼ0まで下がる国が出る(P28-002-PRE-001 §8.3)。
pub const WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS: f64 = 1.0;

/// 戦時MilitaryFactory建設の入力。呼び出し側(`process_daily_country_ai`)が、
/// 戦時のAI国家についてだけ構築する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WartimeSupplyContext {
    /// 現在の国家総需要(`country_daily_demand_units`、units/日)。
    pub daily_demand_units: u128,
}

/// P29-009: Iron の継続的なフロー不足の評価結果。呼び出し側(`process_daily_country_ai`)が、
/// `economy::iron_shortage::IronShortageHistory`が「継続的な不足」と評価した国家についてだけ構築する。
/// 未集計・不足なし・一時的な不足のときは構築しない(従来の判断だけが行われる)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IronSupplyContext {
    /// 直近の窓の月平均の、MilitaryFactory の Iron 未充足の要求量(Iron/月)。
    pub average_unmet: f64,
}

/// Mine を 1 レベル追加したときの Iron 産出の見込み(Iron/月)。州の discovered な Iron 鉱床の
/// 基本出力の合計 × その州の Mine の記録済み稼働率(Mine が未建設・未記録なら`fallback_op`)。
/// 月次生産の Mine 分岐(`出力 × level × 稼働率`)と同じ式の 1 レベル分。
fn iron_mine_yield(state: &crate::state::data::StateData, fallback_op: f32) -> f64 {
    let deposits: f64 = state
        .resource_deposits
        .iter()
        .filter(|d| d.discovered && d.resource_type == ResourceType::Iron)
        .map(|d| d.base_output)
        .sum();
    let op = if state.building_level(BuildingType::Mine) > 0 {
        state
            .building_operation_ratios
            .get(&BuildingType::Mine)
            .copied()
            .unwrap_or(fallback_op)
    } else {
        fallback_op
    };
    deposits * op.clamp(0.0, 1.0) as f64
}

/// 優先度3(平時の任意開発)と同じ予備資金の要求(P27-002-A): 建設費 + 月次経常支出 × 3 か月分
/// を国庫が満たすこと。非有限・負の値は安全側(不許可)。
fn peacetime_reserve_satisfied(country: &CountryData, construction_cost: f64) -> bool {
    let treasury = country.treasury;
    let monthly_expenses = country.monthly_expenses;
    let valid = treasury.is_finite()
        && construction_cost.is_finite()
        && construction_cost >= 0.0
        && monthly_expenses.is_finite()
        && monthly_expenses >= 0.0;
    if !valid {
        return false;
    }
    let required = construction_cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    required.is_finite() && treasury >= required
}

/// P29-010: Factory の増設を抑止するか。日付(`YYYY/MM/DD`)を解釈できない・Factory がない
/// (初回建設)・履歴が不足/古い・設備署名が現在と異なる(完成・増設・所有者変更の直後)・
/// 全 entry が条件を満たさない場合は`false`(従来の建設判断)。
pub fn factory_expansion_blocked(
    history: &crate::economy::factory_starvation::FactoryStarvationHistory,
    current_date_str: &str,
    country_id: CountryId,
    state_registry: &StateRegistry,
) -> bool {
    let mut parts = current_date_str.split('/');
    let (Some(y), Some(m)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(year), Ok(month)) = (y.parse::<i32>(), m.parse::<u8>()) else {
        return false;
    };
    let entries: Vec<(usize, u32)> = state_registry
        .states
        .iter()
        .filter(|s| s.owner_country_id == country_id)
        .map(|s| (s.id.0, s.building_level(BuildingType::Factory)))
        .filter(|(_, l)| *l > 0)
        .collect();
    history.blocks_expansion(
        (year, month),
        crate::economy::factory_starvation::factory_signature(&entries),
        entries.len(),
    )
}

/// P29-009: 履歴から、建設 AI へ渡す Iron 不足の文脈を作る。日付(`YYYY/MM/DD`)を解釈できない
/// ・履歴が未集計・継続的な不足がない場合は`None`。国家の Mine レベル合計は、履歴の各月の値と
/// 比較して「完成・喪失が実績に反映済みか」を判定するために使う。
pub fn iron_supply_context(
    history: &crate::economy::iron_shortage::IronShortageHistory,
    current_date_str: &str,
    country_id: CountryId,
    state_registry: &StateRegistry,
) -> Option<IronSupplyContext> {
    use crate::economy::iron_shortage::IronShortageAssessment;
    let mut parts = current_date_str.split('/');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let mine_levels: u32 = state_registry
        .states
        .iter()
        .filter(|s| s.owner_country_id == country_id)
        .map(|s| s.building_level(BuildingType::Mine))
        .sum();
    match history.assess((year, month), mine_levels) {
        IronShortageAssessment::Sustained { average_unmet } => {
            Some(IronSupplyContext { average_unmet })
        }
        IronShortageAssessment::Unavailable | IronShortageAssessment::NoSustainedShortage => None,
    }
}

/// P29-009: Iron の継続的な不足を補う Mine を 1 レベル着工する。着工したら`true`。
///
/// - 需要: MilitaryFactory の Iron 未充足の要求量の月平均(`context.average_unmet`)。
/// - 供給の見込み: 候補州の Iron 鉱床の基本出力 × Mine の稼働率(1 レベルあたり)。
/// - 建設中の Mine(同一更新中の先行注文を含む)の増分は、不足から差し引く(重複計上しない)。
/// - 残りの不足が 1 レベルの見込みの半分未満なら着工しない(目標を満たした/微小な不足)。
/// - 候補は、Iron の鉱床があり Mine が増設可能で、建設キューに同じ州の Mine がない州のうち、
///   1 レベルあたりの Iron 見込みが最大の州(同値は StateId 最小)。
/// - 予算は優先度3と同じ予備資金の判定。満たさなければ何も変更しない。
fn try_iron_supply_construction(
    country: &mut CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    context: IronSupplyContext,
) -> bool {
    let Some(def) = building_registry.get(BuildingType::Mine) else {
        return false;
    };
    if !context.average_unmet.is_finite() || context.average_unmet <= 0.0 {
        return false;
    }
    // 稼働率が未記録の Mine の見込みには、国内の記録済み Mine 稼働率の平均(なければ 1.0)を使う。
    let (sum, n) = owned_states
        .iter()
        .filter_map(|sid| state_registry.get(*sid))
        .filter(|s| s.building_level(BuildingType::Mine) > 0)
        .filter_map(|s| s.building_operation_ratios.get(&BuildingType::Mine).copied())
        .fold((0.0f32, 0u32), |(a, c), v| (a + v, c + 1));
    let fallback_op = if n > 0 { sum / n as f32 } else { 1.0 };

    let mut best: Option<(StateId, f64)> = None;
    for &sid in owned_states {
        let in_queue = country
            .construction_queue
            .iter()
            .any(|q| q.state_id == sid && q.building_type == BuildingType::Mine);
        if in_queue {
            continue;
        }
        let Some(state) = state_registry.get(sid) else {
            continue;
        };
        if crate::economy::resources::is_crystal_only_state(&state.resource_deposits) {
            continue;
        }
        if state.building_level(BuildingType::Mine) >= def.max_level {
            continue;
        }
        let y = iron_mine_yield(state, fallback_op);
        if y <= 0.0 {
            continue;
        }
        if best.is_none_or(|(_, by)| y > by) {
            best = Some((sid, y));
        }
    }
    let Some((state_id, yield_per_level)) = best else {
        return false;
    };

    let pending: f64 = country
        .construction_queue
        .iter()
        .filter(|q| q.building_type == BuildingType::Mine)
        .filter_map(|q| state_registry.get(q.state_id))
        .map(|s| iron_mine_yield(s, fallback_op))
        .sum();
    let deficit = context.average_unmet - pending;
    if deficit < 0.5 * yield_per_level {
        return false;
    }
    if !peacetime_reserve_satisfied(country, def.construction_cost) {
        return false;
    }
    let current_level = state_registry
        .get(state_id)
        .map(|s| s.building_level(BuildingType::Mine))
        .unwrap_or(0);
    country.treasury -= def.construction_cost;
    country.construction_queue.push(ConstructionQueueItem {
        state_id,
        building_type: BuildingType::Mine,
        target_level: current_level + 1,
        progress: 0.0,
        required_progress: def.required_progress,
        paid_cost: def.construction_cost,
        status: ConstructionStatus::InQueue,
    });
    true
}

/// この州のMilitaryFactoryが前月に実際に生産したMilitarySupplies(月産)。
/// `economy::production::process_country_production`と同じく
/// `出力 × level × 稼働率`で、稼働率は月次生産が書き戻した値をそのまま読む。
fn realised_monthly_military_supplies(
    state: &crate::state::data::StateData,
    def: &crate::building::data::BuildingDefinition,
) -> f64 {
    let output = def.output_resources.get(&ResourceType::MilitarySupplies).copied().unwrap_or(0.0);
    let level = state.building_level(BuildingType::MilitaryFactory);
    output * f64::from(level) * f64::from(state.building_operation(BuildingType::MilitaryFactory))
}

/// この州にMilitaryFactoryを1レベル追加したときの、MilitarySupplies月産の増加見込み。
///
/// 既存の生産式(`稼働率 = min(労働力比, 物流比, 入力充足率)`)のうち、州ごとに
/// 決まる労働力比と物流比だけを、追加前後の建物構成で評価する。入力充足率は国家
/// 単位の備蓄で決まるため、ここでは1.0とし、別途`wartime_supply_inputs_covered`で
/// 1か月分の備蓄を要求する。
/// * 労働力比: `min(州の総必要労働力, 民間労働力) ÷ 労働力`
///   (`population::update`の雇用計算と`calculate_economic_workforce_ratio`をそのまま使う)。
///   追加後は総必要労働力に`required_workforce`を加える。
/// * 物流比: `calculate_logistics_ratio(容量, 使用量)`。追加後は使用量に`logistics_cost`を加える。
///
/// 建設中に人口・雇用・他の建物が変わるため、完成後の実生産はこの見込みと一致する
/// とは限らない(完成後の実生産は次回以降の判定で`realised_monthly_military_supplies`
/// として再評価される)。上限levelの州、労働力0の州、出力を持たない定義は`None`。
pub(crate) fn projected_military_factory_supply_gain(
    state: &crate::state::data::StateData,
    def: &crate::building::data::BuildingDefinition,
    building_registry: &crate::building::data::BuildingRegistry,
) -> Option<f64> {
    let output = *def.output_resources.get(&ResourceType::MilitarySupplies)?;
    let level = state.building_level(BuildingType::MilitaryFactory);
    if level >= def.max_level {
        return None;
    }
    let potential_workforce = state.total_workforce();
    if potential_workforce == 0 {
        return None;
    }
    let civilian_workforce = state.civilian_workforce();
    // `population::update::update_state_population_and_employment`と同じ整数の総必要労働力。
    let required_jobs: u64 = BuildingType::ALL
        .iter()
        .filter_map(|&b| {
            let lv = state.building_level(b);
            (lv > 0)
                .then(|| building_registry.get(b))
                .flatten()
                .map(|d| (d.required_workforce as u64) * u64::from(lv))
        })
        .sum();
    let operation = |jobs: u64, logistics_usage: f32| -> f64 {
        let employed = jobs.min(civilian_workforce);
        let workforce_ratio =
            crate::population::update::calculate_economic_workforce_ratio(employed, potential_workforce)
                .clamp(0.0, 1.0);
        let logistics_ratio =
            crate::logistics::update::calculate_logistics_ratio(state.logistics_capacity, logistics_usage);
        f64::from(workforce_ratio.min(logistics_ratio))
    };
    let before = output * f64::from(level) * operation(required_jobs, state.logistics_usage);
    let after = output
        * f64::from(level + 1)
        * operation(
            required_jobs.saturating_add(def.required_workforce as u64),
            state.logistics_usage + def.logistics_cost,
        );
    Some(after - before)
}

/// MilitaryFactoryを1レベル追加した後の月次入力需要に対し、各入力資源(Iron・
/// IndustrialGoods等、定義から汎用に読む)の国家備蓄が1か月分以上あるか。
///
/// 需要は、この国家が所有する全州(月次生産と同じく`owner_country_id`で数える)の
/// 全建物が、稼働率1.0で消費する量の合計に、追加する1レベル分を加えたもの。実際の
/// 消費は稼働率を掛けた量なので、この判定は実消費より厳しい(保守的な)側にある。
/// 走査は`StateId`昇順・`BuildingType::ALL`順で固定する。
fn wartime_supply_inputs_covered(
    country: &CountryData,
    country_id: CountryId,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    def: &crate::building::data::BuildingDefinition,
) -> bool {
    let mut owned: Vec<&crate::state::data::StateData> =
        state_registry.states.iter().filter(|s| s.owner_country_id == country_id).collect();
    owned.sort_by_key(|s| s.id.0);
    for resource in ResourceType::ALL {
        let Some(&new_level_need) = def.input_resources.get(&resource) else {
            continue;
        };
        let mut monthly_need = new_level_need;
        for state in &owned {
            for building in BuildingType::ALL {
                let level = state.building_level(building);
                if level == 0 {
                    continue;
                }
                if let Some(need) = building_registry.get(building).and_then(|d| d.input_resources.get(&resource)) {
                    monthly_need += need * f64::from(level);
                }
            }
        }
        // 備蓄がNaNの場合も不足として扱う。
        let stock = country.stockpile.get(resource);
        if stock.is_nan() || stock < monthly_need {
            return false;
        }
    }
    true
}

// ─── P28-005: 物流生活水準維持のためのRailway優先建設 ─────────────────────────
//
// `verification_logs/phase-28/p28-005-pre-001/`・`p28-005-pre-002/`で採用した
// R0案。低雇用州に限らず、物流比率が`LOGISTICS_WELFARE_BONUS_RATIO`(0.9)未満の
// 州は`update_state_welfare_and_unrest`の物流ボーナスを受けられず、雇用率<0.7と
// 重なると生活水準が月あたり正味で低下し続け、最終的に税率0.25超のペナルティを
// 誘発して長期の財政崩壊に至る(監査の因果確定)。雇用率で対象州を絞る案(P28-005
// -PRE-002のR1)は、雇用率が境界付近で推移する州で保護が断続的に外れ、新たな
// 崩壊経路を生むことが実測で確認されたため、雇用率による絞り込みは行わない。

/// 優先度2.7(P28-005)。着工した場合だけ`true`(国庫・建設キューを変更する)。
/// `false`のときは国庫・建設キューを一切変更しない。
///
/// 着工条件(すべて満たす場合):
/// 1. 自国が所有かつ支配する陸上州のうち、物流比率が
///    `economy::finance::LOGISTICS_WELFARE_BONUS_RATIO`未満の州がある
///    (雇用率による絞り込みはしない)
/// 2. その州のRailwayが最大レベル未満
/// 3. その州にRailwayが建設キューにない(未完了の同種建設を重複発注しない)
/// 4. 候補が複数あれば物流比率が最も低い州を選ぶ(同値は`StateId`昇順。
///    `owned_states`は呼び出し側で既に`StateId`昇順に整列済みのため、走査順を
///    崩さず「厳密に低い場合だけ更新」する通常の規則で決定的になる)
/// 5. `国庫 ≥ 建設費 + 月次経常支出 × PEACETIME_CONSTRUCTION_RESERVE_MONTHS`
///    (非有限・負の値は着工しない側に倒す。優先度3・戦時MilitaryFactoryと同じ
///    数値安全規約)
fn try_logistics_railway_construction(
    country: &mut CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
) -> bool {
    let Some(def) = building_registry.get(BuildingType::Railway) else {
        return false;
    };

    let mut worst: Option<(f32, StateId, u32)> = None;
    for &state_id in owned_states {
        let Some(state) = state_registry.get(state_id) else {
            continue;
        };
        let level = state.building_level(BuildingType::Railway);
        if level >= def.max_level {
            continue;
        }
        if state.logistics_ratio >= crate::economy::finance::LOGISTICS_WELFARE_BONUS_RATIO {
            continue;
        }
        let queued = country
            .construction_queue
            .iter()
            .any(|item| item.state_id == state_id && item.building_type == BuildingType::Railway);
        if queued {
            continue;
        }
        if worst.is_none_or(|(worst_ratio, _, _)| state.logistics_ratio < worst_ratio) {
            worst = Some((state.logistics_ratio, state_id, level));
        }
    }
    let Some((_, state_id, current_level)) = worst else {
        return false;
    };

    let treasury = country.treasury;
    let cost = def.construction_cost;
    let monthly_expenses = country.monthly_expenses;
    if !(treasury.is_finite()
        && cost.is_finite()
        && cost >= 0.0
        && monthly_expenses.is_finite()
        && monthly_expenses >= 0.0)
    {
        return false;
    }
    let required_treasury = cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
    if !required_treasury.is_finite() || treasury < required_treasury {
        return false;
    }

    country.treasury -= cost;
    country.construction_queue.push(ConstructionQueueItem {
        state_id,
        building_type: BuildingType::Railway,
        target_level: current_level + 1,
        progress: 0.0,
        required_progress: def.required_progress,
        paid_cost: cost,
        status: ConstructionStatus::InQueue,
    });
    true
}

/// 優先度2.5(P28-002)。着工した場合だけ`true`(国庫・建設キューを変更する)。
/// `false`のときは国庫・建設キューを一切変更しない。
///
/// 着工条件(すべて満たす場合):
/// 1. MilitaryFactoryが建設キューにない(未完了の同種建設を重複発注しない)
/// 2. 所有州の実月産(`realised_monthly_military_supplies`の合計) < 日次需要 × 30
///    (MilitarySuppliesの正規units同士で比較する。需要0なら着工しない)
/// 3. 入力資源の備蓄が追加後の月次需要の1か月分以上(`wartime_supply_inputs_covered`)
/// 4. 自国が所有かつ支配する陸上州のうち、増産見込み
///    (`projected_military_factory_supply_gain`)が正で最大の州がある(同値は`StateId`昇順)
/// 5. `国庫 ≥ 建設費 + 月次経常支出 × WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS`
///    (非有限・負の値は着工しない側に倒す。P27-002-Aと同じ数値安全規約)
fn try_wartime_supply_construction(
    country: &mut CountryData,
    country_id: CountryId,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    buildable_states: &[StateId],
    context: WartimeSupplyContext,
) -> bool {
    let Some(def) = building_registry.get(BuildingType::MilitaryFactory) else {
        return false;
    };
    if country
        .construction_queue
        .iter()
        .any(|item| item.building_type == BuildingType::MilitaryFactory)
    {
        return false;
    }

    let Some(monthly_demand_units) = context.daily_demand_units.checked_mul(30) else {
        return false;
    };
    let mut owned: Vec<&crate::state::data::StateData> =
        state_registry.states.iter().filter(|s| s.owner_country_id == country_id).collect();
    owned.sort_by_key(|s| s.id.0);
    let realised: f64 = owned.iter().map(|s| realised_monthly_military_supplies(s, def)).sum();
    let Ok(realised_units) = crate::economy::resources::military_supplies_value_to_units(realised) else {
        return false;
    };
    if u128::from(realised_units) >= monthly_demand_units {
        return false;
    }

    if !wartime_supply_inputs_covered(country, country_id, state_registry, building_registry, def) {
        return false;
    }

    let mut best: Option<(f64, StateId, u32)> = None;
    for &state_id in buildable_states {
        let Some(state) = state_registry.get(state_id) else {
            continue;
        };
        let Some(gain) = projected_military_factory_supply_gain(state, def, building_registry) else {
            continue;
        };
        if gain > 0.0 && best.is_none_or(|(best_gain, _, _)| gain > best_gain) {
            best = Some((gain, state_id, state.building_level(BuildingType::MilitaryFactory)));
        }
    }
    let Some((_, state_id, current_level)) = best else {
        return false;
    };

    let treasury = country.treasury;
    let cost = def.construction_cost;
    let monthly_expenses = country.monthly_expenses;
    if !(treasury.is_finite()
        && cost.is_finite()
        && cost >= 0.0
        && monthly_expenses.is_finite()
        && monthly_expenses >= 0.0)
    {
        return false;
    }
    let required_treasury = cost + monthly_expenses * WARTIME_SUPPLY_CONSTRUCTION_RESERVE_MONTHS;
    if !required_treasury.is_finite() || treasury < required_treasury {
        return false;
    }

    country.treasury -= cost;
    country.construction_queue.push(ConstructionQueueItem {
        state_id,
        building_type: BuildingType::MilitaryFactory,
        target_level: current_level + 1,
        progress: 0.0,
        required_progress: def.required_progress,
        paid_cost: cost,
        status: ConstructionStatus::InQueue,
    });
    true
}

/// 1. 建設AI (月次評価: game_day % 30 == country_id % 30 または dirty)
///
/// 優先順位(P26-004-PRE-002で(2)を新設、`verification_logs/phase-26/p26-004-pre-001/`
/// §20の次タスク定義を踏襲):
/// (1) 食料緊急(Farm) — 既存仕様のまま最優先。
/// (2) 保有する生産建物を支える、未活用の領内鉱床の事前開発(Mine) — 資源備蓄が
///     既存の不足閾値(30)を下回る前に、財政破綻前の開発を狙う(Arcadia)。
/// (3) 既存の通常判断(原材料不足なら鉱山、それ以外は工場)。この段でMineが
///     選ばれたが領内に採掘可能な鉱床が一切ない場合(Elfin/Oceanic等)、Mineへの
///     浪費を防ぐため同じ月のうちに工場建設へ安全にフォールバックする。
///
/// 戦時のAI国家では(2)と(3)の間に戦時MilitaryFactory建設(P28-002)が入る
/// (`process_construction_ai_with_wartime_supply`)。この関数は戦時文脈なしで
/// 呼び出す従来の入口であり、挙動はP28-002以前と同一。
pub fn process_construction_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    ai_state: &mut CountryAiState,
) {
    process_construction_ai_with_wartime_supply(
        country_id,
        country_registry,
        state_registry,
        building_registry,
        ai_state,
        None,
    );
}

/// `process_construction_ai`の本体。`wartime_supply`が`Some`のとき(呼び出し側が
/// 戦時のAI国家についてだけ構築する)、優先度(2)の後・(3)の前で戦時MilitaryFactory
/// 建設(P28-002、`try_wartime_supply_construction`)を評価する。
pub fn process_construction_ai_with_wartime_supply(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    ai_state: &mut CountryAiState,
    wartime_supply: Option<WartimeSupplyContext>,
) {
    process_construction_ai_with_contexts(
        country_id,
        country_registry,
        state_registry,
        building_registry,
        ai_state,
        wartime_supply,
        None,
        false,
    );
}

/// `process_construction_ai_with_wartime_supply`の本体。P29-009: `iron_supply`が`Some`のとき
/// (呼び出し側が、Iron の継続的な不足を履歴から認識できた国家についてだけ構築する)、優先度 2.7
/// (Railway)の後・3 の前で、Iron 供給のための Mine 建設(`try_iron_supply_construction`)を
/// 評価する。着工しなかった場合は優先度 3 へフォールスルーする(従来の判断は変わらない)。
#[allow(clippy::too_many_arguments)]
pub fn process_construction_ai_with_contexts(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    ai_state: &mut CountryAiState,
    wartime_supply: Option<WartimeSupplyContext>,
    iron_supply: Option<IronSupplyContext>,
    factory_expansion_blocked: bool,
) {
    let country = match country_registry.get_mut(country_id) {
        Some(c) => c,
        None => return,
    };

    // 最大2件までの建設キュー制限
    if country.construction_queue.len() >= 2 {
        return;
    }

    // 自国が法的所有かつ実効支配する陸上地域を取得 (StateId昇順)
    let mut owned_states: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.owner_country_id == country_id && s.controller() == country_id)
        .map(|s| s.id)
        .collect();
    owned_states.sort_by_key(|s| s.0);

    if owned_states.is_empty() {
        ai_state.decision_reason = CountryAiDecisionReason::NoBuildableState;
        return;
    }

    // 優先度1: 食料緊急(既存仕様のまま変更しない)
    if country.stockpile.get(ResourceType::Food) < 50.0 {
        ai_state.decision_reason = CountryAiDecisionReason::FoodShortageFarmPriority;
        try_construct(country, state_registry, building_registry, &owned_states, BuildingType::Farm);
        return;
    }

    // 優先度2 (P26-004-PRE-002、P26-008-CでIndustrialGoods/Factoryへ拡張):
    // 保有建物が要求する入力資源のうち、構造的な生産経路(鉱床由来はMine、
    // IndustrialGoodsはFactory)がまだ存在しないものを1件だけ事前開発する。
    // 既存の資源不足トリガー(Wood/Iron/Coal<30)より先に評価する。候補州が
    // 見つかっても、対応する建物の建設費を支払えない場合はこの規則を発火させず
    // (spec §5.3の前提条件を維持)、優先度3へフォールスルーする。
    let mine_max_level = building_registry.get(BuildingType::Mine).map(|d| d.max_level).unwrap_or(0);
    let factory_max_level = building_registry.get(BuildingType::Factory).map(|d| d.max_level).unwrap_or(0);
    if let Some(target) = find_proactive_development_target(
        country,
        state_registry,
        building_registry,
        &owned_states,
        mine_max_level,
        factory_max_level,
    ) {
        let (target_building, state_id, decision_reason) = match target {
            ProactiveDevelopmentTarget::Mine(sid) => {
                (BuildingType::Mine, sid, CountryAiDecisionReason::RawMaterialMinePriority)
            }
            // P26-008-C: 既存のSteelShortageSteelMillPriority(定義済み・
            // localization済みだが、これまで一度も生成側で使われていなかった)を
            // IndustrialGoods不足によるFactory優先の理由として再利用する――
            // 新しいOutcome/reasonを不必要に追加しない(spec §13)。
            ProactiveDevelopmentTarget::Factory(sid) => {
                (BuildingType::Factory, sid, CountryAiDecisionReason::SteelShortageSteelMillPriority)
            }
        };
        // P29-010: 入力不足で低稼働の Factory の増設は、構造的な IG 開発でも除外する(候補なしとして
        // 優先度 2.5 以降へ。予算・キュー枠は消費しない)。
        let factory_excluded =
            target_building == BuildingType::Factory && factory_expansion_blocked;
        if !factory_excluded
            && let Some(def) = building_registry.get(target_building)
            && country.treasury >= def.construction_cost
        {
            ai_state.decision_reason = decision_reason;
            let current_level = state_registry
                .get(state_id)
                .map(|s| s.building_level(target_building))
                .unwrap_or(0);
            country.treasury -= def.construction_cost;
            country.construction_queue.push(ConstructionQueueItem {
                state_id,
                building_type: target_building,
                target_level: current_level + 1,
                progress: 0.0,
                required_progress: def.required_progress,
                paid_cost: def.construction_cost,
                status: ConstructionStatus::InQueue,
            });
            return;
        }
    }

    // 優先度2.5 (P28-002): 戦時のAI国家だけ、MilitarySuppliesの実生産が現在需要に
    // 届かない場合にMilitaryFactoryを1件建設する。着工しなかった場合は、優先度3へ
    // そのままフォールスルーする(優先度3の予備資金契約は変更しない)。
    if let Some(context) = wartime_supply
        && try_wartime_supply_construction(
            country,
            country_id,
            state_registry,
            building_registry,
            &owned_states,
            context,
        )
    {
        ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
        return;
    }

    // 優先度2.7 (P28-005): 物流比率がLOGISTICS_WELFARE_BONUS_RATIO(0.9)未満の
    // 州へRailwayを1件建設する。着工しなかった場合は、優先度3へそのまま
    // フォールスルーする(優先度3の予備資金契約は変更しない)。判断理由は
    // 既存の`PeacetimeDevelopment`を再利用する(Save V1の値域を広げないため、
    // P28-002が`WarInProgress`を再利用したのと同じ方針)。
    if try_logistics_railway_construction(country, state_registry, building_registry, &owned_states) {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        return;
    }

    // 優先度2.8 (P29-009): Iron の継続的なフロー不足が履歴から認識できた場合、不足を補う Mine を
    // 1 件建設する。着工しなかった場合(不足なし・建設中の Mine で目標を満たす・予算不足・候補なし)は、
    // 優先度3へそのままフォールスルーする(無関係な建設判断を止めない)。判断理由は既存の
    // `RawMaterialMinePriority`を再利用する(Save V1 の値域を広げない)。
    if let Some(context) = iron_supply
        && try_iron_supply_construction(country, state_registry, building_registry, &owned_states, context)
    {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        return;
    }

    // 優先度3: 既存の通常判断(原材料不足 -> Mine、それ以外 -> Factory)
    let target_building = if country.stockpile.get(ResourceType::Wood) < 30.0
        || country.stockpile.get(ResourceType::Iron) < 30.0
        || country.stockpile.get(ResourceType::Coal) < 30.0
    {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        BuildingType::Mine
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        BuildingType::Factory
    };

    // P29-010 (B2): 入力不足で低稼働の Factory の増設候補を除外する。優先度 3 の選択肢は Mine か
    // Factory の二択のため、Factory が除外された場合は他の有効候補がなく、建設を保留する
    // (支出を続けるためだけの代替建設はしない。着工・予算・キューは一切変更しない)。
    if target_building == BuildingType::Factory && factory_expansion_blocked {
        return;
    }

    match try_construct_peacetime(country, state_registry, building_registry, &owned_states, target_building) {
        ConstructOutcome::Built => {}
        ConstructOutcome::InsufficientFunds => {
            ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
        }
        ConstructOutcome::NoCandidate => {
            if target_building == BuildingType::Mine {
                // P26-004-PRE-002 spec §5.2: 鉱床皆無国家ではMine候補が存在しない。
                // 同じ月の建設判断を無駄にしないよう、次候補(Factory)へフォールバック
                // する(架空の採掘能力は作らず、既存Factory建設ルールをそのまま再利用)。
                // P27-002-A: このフォールバックも優先度3の一部であり、同じ予備資金
                // 判定(`try_construct_peacetime`)を適用する。
                ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
                if factory_expansion_blocked {
                    return;
                }
                if let ConstructOutcome::InsufficientFunds = try_construct_peacetime(
                    country,
                    state_registry,
                    building_registry,
                    &owned_states,
                    BuildingType::Factory,
                ) {
                    ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
                }
            }
        }
    }
}

/// 2. 研究AI (週次評価: game_day % 7 == country_id % 7 または dirty)
pub fn process_research_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    tech_registry: &TechnologyRegistry,
    world_state: &WorldCivilizationState,
    is_at_war: bool,
    ai_state: &mut CountryAiState,
) {
    let country = match country_registry.get_mut(country_id) {
        Some(c) => c,
        None => return,
    };

    // 利用可能な未研究技術を取得
    let mut available_techs: Vec<&TechnologyDefinition> = tech_registry
        .definitions
        .values()
        .filter(|tech| {
            !country
                .research_state
                .completed_technologies
                .contains(&tech.id)
                && !country.research_state.in_progress.contains_key(&tech.field)
                && tech.minimum_world_stage <= world_state.current_stage
                && tech
                    .prerequisites
                    .iter()
                    .all(|pre| country.research_state.completed_technologies.contains(pre))
        })
        .collect();

    if available_techs.is_empty() {
        ai_state.decision_reason = CountryAiDecisionReason::NoResearchableTech;
        return;
    }

    // 優先順位ソート (コスト昇順 -> ID昇順)
    available_techs.sort_by(|a, b| {
        let a_is_military =
            a.id.contains("military") || a.id.contains("weapon") || a.id.contains("division");
        let b_is_military =
            b.id.contains("military") || b.id.contains("weapon") || b.id.contains("division");

        if is_at_war && a_is_military != b_is_military {
            return b_is_military.cmp(&a_is_military);
        }

        a.cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    if let Some(chosen_tech) = available_techs.first() {
        country.research_state.in_progress.insert(
            chosen_tech.field,
            InProgressTech {
                tech_id: chosen_tech.id.clone(),
                progress: 0.0,
                cost: chosen_tech.cost,
            },
        );
    }
}

/// 3. 戦争準備・正当化AI (週次評価)
///
/// `power_by_country` / `land_states_by_controller` は呼び出し側(`process_daily_country_ai`)が
/// 日次評価の開始時に一度だけ計算した事前計算結果(P20-008最適化)。
/// 本関数はそれ以前と全く同じ値を参照するのみで、判定ロジック・優先順位は変更しない。
#[allow(clippy::too_many_arguments)]
pub fn process_war_preparation_ai(
    current_day: u32,
    country_id: CountryId,
    current_date_str: &str,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    _war_registry: &WarRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    crisis_registry: &CrisisRegistry,
    ai_state: &mut CountryAiState,
    power_by_country: &[u64],
    land_states_by_controller: &[Vec<StateId>],
) {
    // 既存の正当化が進行中かチェック
    let is_justifying = justification_registry
        .justifications
        .values()
        .any(|j| j.initiator == country_id);

    if is_justifying {
        ai_state.mode = CountryAiMode::PreparingWar;
        ai_state.decision_reason = CountryAiDecisionReason::JustificationInProgress;
        return;
    }

    // P21-017(二次レビュー対応): 自国開始の未解決Crisis(まだ正当化が生まれていない
    // DemandSent中も含む)が進行中なら、Crisisと独立した通常の正当化探索を並行して
    // 始めない。Crisis経由でない生の正当化(この関数自身が開始するもの)は、Claim/
    // Crisisシステムと無関係に隣接する到達可能な敵州を選ぶため、放置すると同じ
    // 対象国/州へCrisisと生正当化が二重に進行し得る(例: Crisisが平和的に受諾された
    // 後も、並行して開始されていた生正当化だけが生き残りReadyになって不要な宣戦が
    // 起きる)。
    if crisis_registry.has_unresolved_crisis_initiated_by(country_id) {
        ai_state.mode = CountryAiMode::PreparingWar;
        ai_state.decision_reason = CountryAiDecisionReason::JustificationInProgress;
        return;
    }

    // クールダウン中チェック
    if current_day < ai_state.cooldown_until_day {
        ai_state.decision_reason = CountryAiDecisionReason::PostWarCooldown;
        return;
    }

    // 自国の総有効戦力
    let own_power = total_power_for(power_by_country, country_id);
    if own_power == 0 {
        ai_state.decision_reason = CountryAiDecisionReason::NoAvailableDivisions;
        return;
    }

    // 攻撃対象国候補の抽出 (陸上隣接国)
    let mut candidates: Vec<(CountryId, u64, StateId)> = Vec::new();

    let my_states: &[StateId] = land_states_for(land_states_by_controller, country_id);

    for other_country in &country_registry.countries {
        let tid = other_country.id;
        if tid == country_id {
            continue;
        }

        let target_power = total_power_for(power_by_country, tid);

        // 130% 戦力条件 (own * 1000 >= target * 1300)
        let has_power_advantage = if target_power == 0 {
            own_power > 0
        } else {
            (own_power as u128) * 1000 >= (target_power as u128) * 1300
        };

        if !has_power_advantage {
            continue;
        }

        // 隣接確認 & 到達可能な戦争目標地域の探索
        let enemy_states: &[StateId] = land_states_for(land_states_by_controller, tid);

        let mut valid_target_state: Option<StateId> = None;

        for &esid in enemy_states {
            let reachable = my_states.iter().any(|&msid| {
                find_path(msid, esid, state_registry, &[country_id], &[tid]).is_some()
            });

            if reachable
                && justification_registry
                    .can_start_justification_with_date(
                        country_id,
                        tid,
                        esid,
                        country_registry,
                        state_registry,
                        diplomacy_registry,
                        Some(current_date_str),
                    )
                    .is_ok()
            {
                valid_target_state = Some(esid);
                break;
            }
        }

        if let Some(target_state) = valid_target_state {
            candidates.push((tid, target_power, target_state));
        }
    }

    // 優先度順ソート (対象国戦力が弱く, CountryId最小)
    candidates.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.0.cmp(&b.0.0)));

    if let Some((target_cid, _, target_sid)) = candidates.first() {
        let res = justification_registry.start_justification(
            country_id,
            *target_cid,
            *target_sid,
            current_date_str.to_string(),
            country_registry,
            state_registry,
            diplomacy_registry,
        );

        if res.is_ok() {
            ai_state.mode = CountryAiMode::PreparingWar;
            ai_state.decision_reason = CountryAiDecisionReason::JustificationInProgress;
        }
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::InsufficientPowerAdvantage;
    }
}

/// 4. 宣戦布告AI (日次判定)
#[allow(clippy::too_many_arguments)]
pub fn process_war_declaration_ai(
    country_id: CountryId,
    current_date_str: &str,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &mut WarRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    ai_registry: &mut MilitaryAiRegistry,
    ai_state: &mut CountryAiState,
    pending_ai_wars: &mut Vec<(CountryId, CountryId, StateId, WarId)>,
    crisis_registry: &mut CrisisRegistry,
) {
    let completed_justifications: Vec<(usize, CountryId, StateId)> = justification_registry
        .justifications
        .values()
        .filter(|j| j.initiator == country_id && j.is_ready)
        .map(|j| (j.id, j.target, j.target_state))
        .collect();

    for (j_id, target_cid, target_sid) in completed_justifications {
        let can_declare = war_registry
            .can_declare_war_with_date(
                country_id,
                target_cid,
                target_sid,
                country_registry,
                state_registry,
                diplomacy_registry,
                justification_registry,
                Some(current_date_str),
            )
            .is_ok();

        if can_declare {
            let res = war_registry.declare_war(
                country_id,
                target_cid,
                target_sid,
                current_date_str.to_string(),
                country_registry,
                state_registry,
                diplomacy_registry,
                justification_registry,
            );

            if let Ok(war_id) = res {
                // P21-011: 正当化を削除する前に、Escalating中のCrisisとの照合・同期を行う
                // (declare_war自体が既にconsume_justificationで消費済みだが、
                // ここではidだけを使うため削除順序に依存しない)。
                crisis_response::sync_crisis_on_war_declared(
                    crisis_registry,
                    country_id,
                    target_cid,
                    j_id,
                    war_id,
                );

                // 正当化の削除
                justification_registry.justifications.remove(&j_id);

                // 戦争生成後の前線再構築 & 軍事AI dirty 化
                update_all_frontlines(
                    war_registry,
                    state_registry,
                    military_registry,
                    frontline_registry,
                );
                ai_registry.mark_all_dirty();

                ai_state.mode = CountryAiMode::AtWar;
                ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
                pending_ai_wars.push((country_id, target_cid, target_sid, war_id));
                break;
            }
        }
    }
}

/// 日次国家AI処理のエントリーポイント
#[allow(clippy::too_many_arguments)]
pub fn process_daily_country_ai(
    current_day: u32,
    current_date_str: &str,
    player_country: &PlayerCountry,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &mut WarRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    tech_registry: &TechnologyRegistry,
    world_state: &WorldCivilizationState,
    justification_registry: &mut WarJustificationRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    military_ai_registry: &mut MilitaryAiRegistry,
    country_ai_registry: &mut CountryAiRegistry,
    pending_ai_wars: &mut Vec<(CountryId, CountryId, StateId, WarId)>,
    crisis_registry: &mut CrisisRegistry,
) {
    let player_cid = player_country.0;

    let mut ai_country_ids: Vec<CountryId> = country_registry
        .countries
        .iter()
        .filter(|c| c.is_active())
        .map(|c| c.id)
        .filter(|&cid| Some(cid) != player_cid)
        .collect();
    ai_country_ids.sort_by_key(|c| c.0);

    // P20-008最適化: 週次の戦争準備AI (process_war_preparation_ai) が候補国ごとに
    // 総戦力・支配State一覧を再計算していたO(countries)重複を排除するため、
    // 日次評価の開始時に一度だけ計算する。値は再計算した場合と完全に一致する
    // (`compute_total_power_by_country` / `compute_land_states_by_controller` のdoc参照)。
    //
    // P20-008追補: 全戦争中で `process_war_preparation_ai` が1件も呼ばれない日
    // (例: 全AI国家が交戦中)には構築自体を省略するため、`Option`で遅延構築する。
    // ループ内で実際に必要になった最初の国の処理時に一度だけ構築し、同日内の
    // 以降の国では再利用する(値・優先順位は事前構築時と完全に同一)。
    let mut power_by_country: Option<Vec<u64>> = None;
    let mut land_states_by_controller: Option<Vec<Vec<StateId>>> = None;

    for country_id in ai_country_ids {
        let is_at_war = war_registry.wars.values().any(|w| {
            w.status == WarStatus::Active
                && (w.attackers.contains(&country_id) || w.defenders.contains(&country_id))
        });

        // P29-009: 建設 AI の Iron 不足認識用の履歴(ai_state の借用の前に複製する)。
        let iron_history = country_ai_registry
            .iron_shortage_history
            .get(&country_id)
            .cloned()
            .unwrap_or_default();
        // P29-010: Factory 増設抑止(B2)用の履歴(同上)。
        let factory_history = country_ai_registry
            .factory_starvation_history
            .get(&country_id)
            .cloned()
            .unwrap_or_default();

        let ai_state = country_ai_registry.get_or_create_mut(country_id);

        // 高位モードの設定
        if is_at_war {
            ai_state.mode = CountryAiMode::AtWar;
            ai_state.decision_reason = CountryAiDecisionReason::WarInProgress;
        } else if ai_state.mode == CountryAiMode::AtWar {
            ai_state.mode = CountryAiMode::Recovering;
            ai_state.decision_reason = CountryAiDecisionReason::PostWarCooldown;
        }

        // 1. 日次宣戦布告AI
        process_war_declaration_ai(
            country_id,
            current_date_str,
            country_registry,
            state_registry,
            military_registry,
            war_registry,
            diplomacy_registry,
            justification_registry,
            frontline_registry,
            military_ai_registry,
            ai_state,
            pending_ai_wars,
            crisis_registry,
        );

        // 2. 週次評価 (game_day % 7 == country_id.0 % 7 または dirty)
        let is_weekly_due = (current_day as usize % 7 == country_id.0 % 7) || ai_state.dirty;
        if is_weekly_due {
            process_research_ai(
                country_id,
                country_registry,
                tech_registry,
                world_state,
                is_at_war,
                ai_state,
            );

            if !is_at_war {
                let power_map = power_by_country.get_or_insert_with(|| {
                    compute_total_power_by_country(military_registry, state_registry)
                });
                let land_map = land_states_by_controller
                    .get_or_insert_with(|| compute_land_states_by_controller(state_registry));

                process_war_preparation_ai(
                    current_day,
                    country_id,
                    current_date_str,
                    country_registry,
                    state_registry,
                    war_registry,
                    diplomacy_registry,
                    justification_registry,
                    &*crisis_registry,
                    ai_state,
                    power_map.as_slice(),
                    land_map.as_slice(),
                );
            }
            ai_state.last_weekly_evaluation_day = current_day;
        }

        // 3. 月次評価 (game_day % 30 == country_id.0 % 30 または dirty)
        let is_monthly_due = (current_day as usize % 30 == country_id.0 % 30) || ai_state.dirty;
        if is_monthly_due {
            // P27-002-B: 税率評価を建設評価より先に行う(spec §7の順序)。ただし
            // このSystem自体は既にDailySimulationSet::Economyの後(CountryAi)で
            // 動くため、当月分のtax_rate変更が同月の税収へ遡及することはなく、
            // 次回の月次財政計算(Economy Set)から新税率が使われる。
            process_tax_rate_ai(country_id, country_registry, state_registry);

            // P28-002: 戦時のAI国家だけに戦時MilitaryFactory建設の文脈を渡す
            // (このループはプレイヤー国家・消滅国家を既に除外している)。需要の算出が
            // 失敗する防御的なケースでは文脈を作らず、従来の建設判断だけを行う。
            let wartime_supply = if is_at_war {
                crate::military::supply_initialization::country_daily_demand_units(
                    country_id,
                    &military_registry.divisions,
                    &military_registry.definitions,
                )
                .ok()
                .map(|daily_demand_units| WartimeSupplyContext { daily_demand_units })
            } else {
                None
            };
            // P29-009: Iron の継続的なフロー不足が履歴から認識できた国家にだけ文脈を渡す。
            // 未集計・不足なし・一時的な不足のときは`None`(従来の判断)。
            let iron_supply = iron_supply_context(
                &iron_history,
                current_date_str,
                country_id,
                state_registry,
            );
            process_construction_ai_with_contexts(
                country_id,
                country_registry,
                state_registry,
                building_registry,
                ai_state,
                wartime_supply,
                iron_supply,
                factory_expansion_blocked(
                    &factory_history,
                    current_date_str,
                    country_id,
                    state_registry,
                ),
            );
            ai_state.last_monthly_evaluation_day = current_day;
        }

        ai_state.last_daily_evaluation_day = current_day;
        ai_state.dirty = false;
    }
}

/// System for daily country AI evaluation
#[allow(clippy::too_many_arguments)]
pub fn handle_daily_country_ai(
    mut day_events: MessageReader<DayChangedMessage>,
    player_country: Res<PlayerCountry>,
    mut country_registry: ResMut<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    building_registry: Res<crate::building::data::BuildingRegistry>,
    military_registry: Res<MilitaryRegistry>,
    mut war_registry: ResMut<WarRegistry>,
    mut diplomacy_registry: ResMut<DiplomacyRegistry>,
    tech_registry: Res<TechnologyRegistry>,
    world_state: Res<WorldCivilizationState>,
    mut justification_registry: ResMut<WarJustificationRegistry>,
    mut frontline_registry: ResMut<crate::war::frontline::FrontlineRegistry>,
    mut military_ai_registry: ResMut<MilitaryAiRegistry>,
    mut country_ai_registry: ResMut<CountryAiRegistry>,
    mut pending_ai_wars: ResMut<PendingAiWarDeclarations>,
    mut crisis_registry: ResMut<CrisisRegistry>,
) {
    for event in day_events.read() {
        let current_date = format!("{:04}/{:02}/{:02}", event.year, event.month, event.day);
        let current_day_num =
            (event.year * 365 + event.month as i32 * 30 + event.day as i32) as u32;

        process_daily_country_ai(
            current_day_num,
            &current_date,
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut pending_ai_wars.0,
            &mut crisis_registry,
        );
    }
}

/// AI国家同士(またはAI国家がプレイヤーに対して)宣戦布告した際に、
/// プレイヤーの宣戦布告(`diplomacy_panel.rs`)と同様の通知/ログを発行する。
/// `handle_daily_country_ai`本体とは別Systemに分離しているのは、既にSystemParamの
/// タプル数上限に近い同Systemへローカライズ関連の追加パラメータを増やさないため
/// (詳細は`Loc`のdocコメントを参照)。
pub fn emit_ai_war_declaration_notifications(
    mut pending: ResMut<PendingAiWarDeclarations>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    loc: Loc,
    mut notif_writer: MessageWriter<GameNotification>,
) {
    if pending.0.is_empty() {
        return;
    }

    for (attacker, defender, target_state, war_id) in std::mem::take(&mut pending.0) {
        let attacker_name = country_registry
            .get(attacker)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| loc.t("common.unknown"));
        let defender_name = country_registry
            .get(defender)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| loc.t("common.unknown"));
        let state_name = state_registry
            .get(target_state)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| loc.t("common.unknown"));

        notif_writer.write(GameNotification {
            message: tf(
                &loc.catalog,
                loc.locale.0,
                "notif.ai_war_declared",
                vec![
                    ("attacker", attacker_name),
                    ("defender", defender_name),
                    ("state", state_name),
                    ("war_id", format!("{:?}", war_id)),
                ],
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::BuildingType;
    use crate::country::CountryData;
    use crate::state::data::StateData;
    use std::collections::HashMap;

    #[allow(clippy::type_complexity)]
    fn setup_test_env() -> (
        PlayerCountry,
        CountryRegistry,
        StateRegistry,
        crate::building::data::BuildingRegistry,
        WarRegistry,
        MilitaryRegistry,
        DiplomacyRegistry,
        TechnologyRegistry,
        WorldCivilizationState,
        WarJustificationRegistry,
        crate::war::frontline::FrontlineRegistry,
        MilitaryAiRegistry,
        CountryAiRegistry,
    ) {
        let player_country = PlayerCountry(Some(CountryId(1)));

        let c1 = CountryData {
            id: CountryId(1),
            name: "Player C1".to_string(),
            capital_state_id: StateId(1),
            treasury: 1000.0,
            ..default()
        };
        let c2 = CountryData {
            id: CountryId(2),
            name: "AI C2".to_string(),
            capital_state_id: StateId(3),
            treasury: 1000.0,
            ..default()
        };

        let country_registry = CountryRegistry {
            countries: vec![c1, c2],
        };

        let s1 = StateData {
            id: StateId(1),
            name: "State 1".to_string(),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s2 = StateData {
            id: StateId(2),
            name: "State 2".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1), StateId(3)],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "State 3".to_string(),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(2)],
            world_position: [200.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };

        let state_registry = StateRegistry::build(vec![s1, s2, s3]);
        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry.definitions.insert(
            BuildingType::Farm,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Farm,
                name: "Farm".to_string(),
                construction_cost: 100.0,
                required_progress: 10.0,
                required_workforce: 100.0,
                logistics_cost: 0.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 1.0,
                max_level: 5,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );

        (
            player_country,
            country_registry,
            state_registry,
            building_registry,
            WarRegistry::default(),
            MilitaryRegistry::default(),
            DiplomacyRegistry::default(),
            TechnologyRegistry::default(),
            WorldCivilizationState::default(),
            WarJustificationRegistry::default(),
            crate::war::frontline::FrontlineRegistry::default(),
            MilitaryAiRegistry::default(),
            CountryAiRegistry::default(),
        )
    }

    #[test]
    fn test_player_country_not_controlled_by_country_ai() {
        let (
            player_country,
            mut country_registry,
            state_registry,
            building_registry,
            mut war_registry,
            military_registry,
            mut diplomacy_registry,
            tech_registry,
            world_state,
            mut justification_registry,
            mut frontline_registry,
            mut military_ai_registry,
            mut country_ai_registry,
        ) = setup_test_env();

        let mut pending_ai_wars = Vec::new();
        let mut crisis_registry = CrisisRegistry::default();
        process_daily_country_ai(
            1,
            "1800/01/01",
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut pending_ai_wars,
            &mut crisis_registry,
        );

        // プレイヤー国家 (CountryId(1)) は国家AI状態が作成されない
        assert!(!country_ai_registry.ai_states.contains_key(&CountryId(1)));

        // AI国家 (CountryId(2)) は作成される
        assert!(country_ai_registry.ai_states.contains_key(&CountryId(2)));
    }

    #[test]
    fn test_ai_construction_validity_and_cost() {
        let (
            _,
            mut country_registry,
            state_registry,
            building_registry,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            mut country_ai_registry,
        ) = setup_test_env();

        let ai_state = country_ai_registry.get_or_create_mut(CountryId(2));
        let initial_treasury = country_registry.get(CountryId(2)).unwrap().treasury;

        process_construction_ai(
            CountryId(2),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c2 = country_registry.get(CountryId(2)).unwrap();
        // 建設キューに1件追加され、資金が減っていることを確認
        assert_eq!(c2.construction_queue.len(), 1);
        assert!(c2.treasury < initial_treasury);
    }

    /// P21-009-FIX-001要求テスト項目7: AIもクリスタル専用州へ通常Mineを新規建設しない
    /// (2州所有し、片方はクリスタル専用・もう片方は非クリスタルの場合、後者が選ばれる)。
    #[test]
    fn ai_construction_skips_crystal_only_state_for_mine() {
        use crate::economy::resources::{ResourceType as Res, StateResourceDeposit};

        let mut c2 = CountryData {
            id: CountryId(2),
            name: "AI C2".to_string(),
            capital_state_id: StateId(3),
            treasury: 5000.0,
            ..default()
        };
        // FoodはFarm優先を避けるため十分、Wood/Iron/Coalは低くMine優先を発火させる。
        c2.stockpile.set(Res::Food, 100.0);
        c2.stockpile.set(Res::Wood, 0.0);
        c2.stockpile.set(Res::Iron, 0.0);
        c2.stockpile.set(Res::Coal, 0.0);
        let mut country_registry = CountryRegistry {
            countries: vec![c2],
        };

        // StateId(2)はクリスタル専用(MagicCrystal鉱床のみ)、StateId(3)は非クリスタル。
        let s2 = StateData {
            id: StateId(2),
            name: "Crystal Only".to_string(),
            owner_country_id: CountryId(2),
            resource_deposits: vec![StateResourceDeposit {
                resource_type: Res::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            }],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let s3 = StateData {
            id: StateId(3),
            name: "Iron State".to_string(),
            owner_country_id: CountryId(2),
            resource_deposits: vec![StateResourceDeposit {
                resource_type: Res::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            }],
            world_position: [100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        let state_registry = StateRegistry::build(vec![s2, s3]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry.definitions.insert(
            BuildingType::Mine,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Mine,
                name: "Mine".to_string(),
                construction_cost: 500.0,
                required_progress: 60.0,
                required_workforce: 10_000.0,
                logistics_cost: 10.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 15.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );

        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(2));

        process_construction_ai(
            CountryId(2),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c2 = country_registry.get(CountryId(2)).unwrap();
        assert_eq!(
            ai_state.decision_reason,
            CountryAiDecisionReason::RawMaterialMinePriority
        );
        assert_eq!(c2.construction_queue.len(), 1);
        assert_eq!(c2.construction_queue[0].building_type, BuildingType::Mine);
        assert_eq!(
            c2.construction_queue[0].state_id,
            StateId(3),
            "AI must pick the non-crystal state, skipping the crystal-only StateId(2)"
        );
    }

    // ── P26-004-PRE-002: 鉱床を考慮したMine建設AI ─────────────────────────────
    // `verification_logs/phase-26/p26-004-pre-002/` タスクの必須検証シナリオ(spec §7
    // Test A〜I相当)を単体テストとして固定する。実App・実データによるArcadia/Elfin/
    // Oceanic検証(Test J・K)は`tests/p26_004_pre_002_deposit_aware_mine_construction_ai_e2e_test.rs`
    // 側で行う。

    use crate::economy::resources::{ResourceType as Res, StateResourceDeposit};

    fn mine_def() -> crate::building::data::BuildingDefinition {
        crate::building::data::BuildingDefinition {
            building_type: BuildingType::Mine,
            name: "Mine".to_string(),
            construction_cost: 500.0,
            required_progress: 60.0,
            required_workforce: 10_000.0,
            logistics_cost: 10.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::new(),
            maintenance_cost: 15.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn factory_def() -> crate::building::data::BuildingDefinition {
        crate::building::data::BuildingDefinition {
            building_type: BuildingType::Factory,
            name: "Civilian Factory".to_string(),
            construction_cost: 1000.0,
            required_progress: 90.0,
            required_workforce: 20_000.0,
            logistics_cost: 15.0,
            input_resources: HashMap::from([(Res::Iron, 20.0), (Res::Coal, 20.0)]),
            output_resources: HashMap::from([(Res::IndustrialGoods, 30.0)]),
            maintenance_cost: 30.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn military_factory_def() -> crate::building::data::BuildingDefinition {
        crate::building::data::BuildingDefinition {
            building_type: BuildingType::MilitaryFactory,
            name: "Military Factory".to_string(),
            construction_cost: 1200.0,
            required_progress: 90.0,
            required_workforce: 25_000.0,
            logistics_cost: 20.0,
            input_resources: HashMap::from([(Res::Iron, 30.0), (Res::IndustrialGoods, 15.0)]),
            output_resources: HashMap::from([
                (Res::MilitaryEquipment, 20.0),
                (Res::MilitarySupplies, 100.0),
            ]),
            maintenance_cost: 40.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn no_deposit_state(id: StateId, owner: CountryId) -> StateData {
        StateData {
            id,
            name: format!("State {}", id.0),
            owner_country_id: owner,
            world_position: [id.0 as f32 * 100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        }
    }

    fn iron_coal_deposit_state(id: StateId, owner: CountryId) -> StateData {
        StateData {
            id,
            name: format!("State {}", id.0),
            owner_country_id: owner,
            resource_deposits: vec![
                StateResourceDeposit {
                    resource_type: Res::Iron,
                    base_output: 50.0,
                    discovered: true,
                    development_level: 0,
                },
                StateResourceDeposit {
                    resource_type: Res::Coal,
                    base_output: 40.0,
                    discovered: true,
                    development_level: 0,
                },
            ],
            world_position: [id.0 as f32 * 100.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        }
    }

    fn healthy_stockpile(country: &mut CountryData) {
        country.stockpile.set(Res::Food, 100.0);
        country.stockpile.set(Res::Wood, 100.0);
        country.stockpile.set(Res::Iron, 100.0);
        country.stockpile.set(Res::Coal, 100.0);
    }

    /// Test A: 領土内に鉱床が一切ない国家は、資源が不足していてもMineを建設せず、
    /// 既存の次候補(Factory)へ安全にフォールバックする。国庫はFactory分の正規費用
    /// だけ減り、他国家・panicへの影響はない。
    #[test]
    fn zero_deposit_country_never_builds_mine_and_falls_back_to_factory() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "Elfin-like".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        c1.stockpile.set(Res::Food, 100.0);
        c1.stockpile.set(Res::Wood, 0.0);
        c1.stockpile.set(Res::Iron, 0.0);
        c1.stockpile.set(Res::Coal, 0.0);
        let initial_treasury = c1.treasury;
        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };

        let s1 = no_deposit_state(StateId(1), CountryId(1));
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());

        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(
            c1.construction_queue[0].building_type,
            BuildingType::Factory,
            "no usable deposit anywhere: Mine must never be chosen"
        );
        assert_eq!(
            c1.treasury,
            initial_treasury - factory_def().construction_cost,
            "treasury must decrease by exactly the Factory's own cost, never a Mine cost"
        );
    }

    /// Test A続き: FactoryもMineも建てられない(建物レジストリにMineしかない)場合、
    /// 鉱床皆無国家はMineレベル不変・国庫不変のまま何も建設しない(panicなし)。
    #[test]
    fn zero_deposit_country_with_no_fallback_building_builds_nothing() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "Elfin-like".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        c1.stockpile.set(Res::Food, 100.0);
        c1.stockpile.set(Res::Wood, 0.0);
        c1.stockpile.set(Res::Iron, 0.0);
        c1.stockpile.set(Res::Coal, 0.0);
        let initial_treasury = c1.treasury;
        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };

        let s1 = no_deposit_state(StateId(1), CountryId(1));
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());

        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 0);
        assert_eq!(c1.treasury, initial_treasury);
        assert_eq!(state_registry.get(StateId(1)).unwrap().building_level(BuildingType::Mine), 0);
    }

    /// Test B: 同一国家内に「鉱床なし州」と「鉱床あり州」がある場合、Mineは必ず
    /// 鉱床のある州に建設される。
    #[test]
    fn mine_is_built_on_the_state_with_a_deposit_not_the_empty_one() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "C1".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        c1.stockpile.set(Res::Food, 100.0);
        c1.stockpile.set(Res::Wood, 0.0);
        c1.stockpile.set(Res::Iron, 0.0);
        c1.stockpile.set(Res::Coal, 0.0);
        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };

        // StateId(1)は鉱床なし、StateId(2)はIron/Coal鉱床あり。
        // StateId昇順(1→2)の走査でも、鉱床のないState 1が誤って選ばれないことを確認。
        let s1 = no_deposit_state(StateId(1), CountryId(1));
        let s2 = iron_coal_deposit_state(StateId(2), CountryId(1));
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());

        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(c1.construction_queue[0].building_type, BuildingType::Mine);
        assert_eq!(c1.construction_queue[0].state_id, StateId(2));
    }

    /// Test C: MilitaryFactoryの入力資源(Iron)を必要とする生産建物を保有し、
    /// 現時点の備蓄は既存の不足閾値(30)以上、対応する稼働中Mineはなく、対応する
    /// 未活用鉱床を領内に持ち、建設費を支払える国家では、通常のFactory拡張より先に
    /// Mineが選ばれる(Arcadia型シナリオの単体再現、事前開発トリガー)。
    #[test]
    fn proactive_mine_development_fires_before_the_reactive_stockpile_threshold() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "Arcadia-like".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        // 資源備蓄は既存閾値(30)を上回っている: 事前開発規則がなければ Factory が
        // 選ばれてしまう状態。
        healthy_stockpile(&mut c1);
        let mut s1 = iron_coal_deposit_state(StateId(1), CountryId(1));
        // 既にMilitaryFactory(lv1)を保有: Ironを入力として要求するが、稼働中Mineはまだない。
        s1.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());
        building_registry
            .definitions
            .insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };
        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(
            c1.construction_queue[0].building_type,
            BuildingType::Mine,
            "an unmet Iron input for an owned MilitaryFactory must trigger proactive Mine \
             development ahead of the normal Factory choice, even though stockpiles are healthy"
        );
        assert_eq!(c1.construction_queue[0].state_id, StateId(1));
        assert_eq!(
            ai_state.decision_reason,
            CountryAiDecisionReason::RawMaterialMinePriority
        );
    }

    /// Test D: 生産建物が必要とする資源と一致しない鉱床(MagicCrystalのみ)しかない
    /// 場合、事前開発規則はその鉱床を選ばない(通常のクリスタル専用州除外と合わせて、
    /// 結果的にMineは一切建設されない)。
    #[test]
    fn proactive_rule_ignores_a_deposit_that_does_not_match_the_needed_resource() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "C1".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        healthy_stockpile(&mut c1);
        let mut s1 = StateData {
            id: StateId(1),
            name: "Crystal Only".to_string(),
            owner_country_id: CountryId(1),
            resource_deposits: vec![StateResourceDeposit {
                resource_type: Res::MagicCrystal,
                base_output: 30.0,
                discovered: true,
                development_level: 1,
            }],
            world_position: [0.0, 0.0],
            size: [100.0, 100.0],
            ..default()
        };
        // MilitaryFactory(lv1)はIron/IndustrialGoodsを要求するが、この州の鉱床は
        // MagicCrystalのみで一致しない。
        s1.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());
        building_registry
            .definitions
            .insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };
        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(
            c1.construction_queue[0].building_type,
            BuildingType::Factory,
            "an unrelated MagicCrystal-only deposit must not be treated as satisfying the \
             Iron/IndustrialGoods need, and must not itself be built on (crystal-only exclusion)"
        );
    }

    /// Test E: 必要資源(Iron/Coal)について既に稼働中のMine(Dwarf型)がある国家では、
    /// 事前開発規則が不必要な追加Mineを強制しない(既存供給網を壊さない)。
    #[test]
    fn proactive_rule_does_not_force_an_extra_mine_when_supply_is_already_active() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "Dwarf-like".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        healthy_stockpile(&mut c1);
        let mut s1 = iron_coal_deposit_state(StateId(1), CountryId(1));
        // 鉱床と同居する稼働中Mine(lv5)。
        s1.buildings.insert(BuildingType::Mine, 5);
        s1.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());
        building_registry
            .definitions
            .insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };
        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(
            c1.construction_queue[0].building_type,
            BuildingType::Factory,
            "Iron/Coal are already actively mined at State 1, so the proactive rule must not \
             fire; the normal healthy-stockpile path (Factory) applies instead"
        );
    }

    /// Test F: 適切な鉱床があっても既存の建設費を支払えない場合、Mineを建設しない・
    /// 国庫を変更しない・他建物を部分更新しない。
    #[test]
    fn insufficient_funds_blocks_proactive_mine_without_any_partial_update() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "PoorArcadia-like".to_string(),
            capital_state_id: StateId(1),
            // Mine建設費(500.0)未満。
            treasury: 100.0,
            ..default()
        };
        healthy_stockpile(&mut c1);
        let initial_treasury = c1.treasury;
        let mut s1 = iron_coal_deposit_state(StateId(1), CountryId(1));
        s1.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());
        building_registry
            .definitions
            .insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };
        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.construction_queue.len(), 0);
        assert_eq!(c1.treasury, initial_treasury);
    }

    /// Test G: 鉱床州のMineが既に上限(max_level)なら、事前開発・通常経路いずれの
    /// 候補からも除外される。
    #[test]
    fn mine_at_max_level_is_excluded_from_every_candidate_path() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "C1".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        c1.stockpile.set(Res::Food, 100.0);
        c1.stockpile.set(Res::Wood, 0.0);
        c1.stockpile.set(Res::Iron, 0.0);
        c1.stockpile.set(Res::Coal, 0.0);
        let mut s1 = iron_coal_deposit_state(StateId(1), CountryId(1));
        s1.buildings.insert(BuildingType::Mine, 10); // max_level=10
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());

        let mut country_registry = CountryRegistry {
            countries: vec![c1],
        };
        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(
            c1.construction_queue.len(),
            0,
            "a Mine already at max_level must not be selected again by any path"
        );
    }

    /// Test H: 同じ論理状態を異なる州挿入順で構築しても、同じ資源・同じStateId・
    /// 同じ建物が選ばれる(HashMap/Vec挿入順に依存しない決定性)。
    #[test]
    fn candidate_selection_is_deterministic_regardless_of_state_insertion_order() {
        fn run_with_order(states: Vec<StateData>) -> Option<StateId> {
            let mut c1 = CountryData {
                id: CountryId(1),
                name: "C1".to_string(),
                capital_state_id: StateId(1),
                treasury: 5000.0,
                ..default()
            };
            healthy_stockpile(&mut c1);
            let state_registry = StateRegistry::build(states);

            let mut building_registry = crate::building::data::BuildingRegistry::default();
            building_registry
                .definitions
                .insert(BuildingType::Mine, mine_def());
            building_registry
                .definitions
                .insert(BuildingType::Factory, factory_def());
            building_registry
                .definitions
                .insert(BuildingType::MilitaryFactory, military_factory_def());

            let mut country_registry = CountryRegistry {
                countries: vec![c1],
            };
            let mut country_ai_registry = CountryAiRegistry::default();
            let ai_state = country_ai_registry.get_or_create_mut(CountryId(1));

            process_construction_ai(
                CountryId(1),
                &mut country_registry,
                &state_registry,
                &building_registry,
                ai_state,
            );

            let c1 = country_registry.get(CountryId(1)).unwrap();
            assert_eq!(c1.construction_queue.len(), 1);
            assert_eq!(c1.construction_queue[0].building_type, BuildingType::Mine);
            Some(c1.construction_queue[0].state_id)
        }

        let mut low = iron_coal_deposit_state(StateId(1), CountryId(1));
        low.buildings.insert(BuildingType::MilitaryFactory, 1);
        let high = iron_coal_deposit_state(StateId(5), CountryId(1));

        let order_a = run_with_order(vec![low.clone(), high.clone()]);
        let order_b = run_with_order(vec![high, low]);

        assert_eq!(order_a, order_b);
        assert_eq!(order_a, Some(StateId(1)), "StateId ascending must win the tie-break");
    }

    /// Test I: 複数国家を同時に処理しても、一国の鉱床・資金・建設失敗が他国へ影響しない。
    #[test]
    fn construction_decisions_are_isolated_across_countries() {
        let mut c1 = CountryData {
            id: CountryId(1),
            name: "ZeroDeposit".to_string(),
            capital_state_id: StateId(1),
            treasury: 5000.0,
            ..default()
        };
        c1.stockpile.set(Res::Food, 100.0);
        c1.stockpile.set(Res::Wood, 0.0);
        c1.stockpile.set(Res::Iron, 0.0);
        c1.stockpile.set(Res::Coal, 0.0);

        let mut c2 = CountryData {
            id: CountryId(2),
            name: "HasDeposit".to_string(),
            capital_state_id: StateId(2),
            treasury: 5000.0,
            ..default()
        };
        healthy_stockpile(&mut c2);
        let c2_initial_treasury = c2.treasury;

        let mut country_registry = CountryRegistry {
            countries: vec![c1, c2],
        };

        let s1 = no_deposit_state(StateId(1), CountryId(1));
        let mut s2 = iron_coal_deposit_state(StateId(2), CountryId(2));
        s2.buildings.insert(BuildingType::MilitaryFactory, 1);
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry
            .definitions
            .insert(BuildingType::Mine, mine_def());
        building_registry
            .definitions
            .insert(BuildingType::Factory, factory_def());
        building_registry
            .definitions
            .insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_ai_registry = CountryAiRegistry::default();
        let ai_state_1 = country_ai_registry.get_or_create_mut(CountryId(1));
        process_construction_ai(
            CountryId(1),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state_1,
        );
        let ai_state_2 = country_ai_registry.get_or_create_mut(CountryId(2));
        process_construction_ai(
            CountryId(2),
            &mut country_registry,
            &state_registry,
            &building_registry,
            ai_state_2,
        );

        let c1 = country_registry.get(CountryId(1)).unwrap();
        let c2 = country_registry.get(CountryId(2)).unwrap();
        assert_eq!(c1.construction_queue.len(), 1);
        assert_eq!(c1.construction_queue[0].building_type, BuildingType::Factory);
        assert_eq!(c2.construction_queue.len(), 1);
        assert_eq!(c2.construction_queue[0].building_type, BuildingType::Mine);
        assert_eq!(c2.construction_queue[0].state_id, StateId(2));
        assert_eq!(
            c2.treasury,
            c2_initial_treasury - mine_def().construction_cost
        );
    }

    // ── P20-008 回帰テスト ───────────────────────────────────────────────────
    // `process_war_preparation_ai` のO(countries)重複計算を事前計算マップに
    // 置き換えた最適化(P20-008)が、既存の`calculate_country_total_power`単体呼び出し
    // および州支配フィルタと完全に同一の値を返すことを検証する。

    fn make_test_division(
        owner: CountryId,
        current_state: StateId,
        manpower: u64,
    ) -> crate::military::data::Division {
        use crate::military::data::{DivisionSize, DivisionStatus, DivisionType};
        crate::military::data::Division {
            id: crate::common::DivisionId(0),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state,
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: manpower,
            equipment: 10.0,
            max_equipment: 10.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 1.0,
            max_morale: 1.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: crate::common::DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    #[test]
    fn test_compute_total_power_by_country_matches_individual_calculation() {
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(2)],
            ..default()
        };
        let s2 = StateData {
            id: StateId(2),
            owner_country_id: CountryId(2),
            neighbors: vec![StateId(1)],
            ..default()
        };
        let state_registry = StateRegistry::build(vec![s1, s2]);

        let mut military_registry = MilitaryRegistry::default();
        military_registry.add_division(make_test_division(CountryId(1), StateId(1), 1000));
        military_registry.add_division(make_test_division(CountryId(1), StateId(1), 500));
        military_registry.add_division(make_test_division(CountryId(2), StateId(2), 2000));

        let precomputed = compute_total_power_by_country(&military_registry, &state_registry);

        for cid in [CountryId(1), CountryId(2), CountryId(3)] {
            let expected = calculate_country_total_power(cid, &military_registry, &state_registry);
            let actual = total_power_for(&precomputed, cid);
            assert_eq!(
                actual, expected,
                "事前計算マップの国{cid:?}の総戦力は個別計算(calculate_country_total_power)と完全に一致すること"
            );
        }
    }

    #[test]
    fn test_compute_land_states_by_controller_matches_individual_filter() {
        let (_, _, state_registry, _, _, _, _, _, _, _, _, _, _) = setup_test_env();

        let precomputed = compute_land_states_by_controller(&state_registry);

        for country_id in [CountryId(1), CountryId(2)] {
            let mut expected: Vec<StateId> = state_registry
                .states
                .iter()
                .filter(|s| !s.is_sea && s.controller() == country_id)
                .map(|s| s.id)
                .collect();
            let mut actual = land_states_for(&precomputed, country_id).to_vec();
            expected.sort_by_key(|s| s.0);
            actual.sort_by_key(|s| s.0);
            assert_eq!(
                actual, expected,
                "事前計算マップの国{country_id:?}の支配State一覧は個別フィルタ結果と完全に一致すること"
            );
        }
    }

    /// 最適化後の `process_war_preparation_ai` (事前計算マップ経由)が、
    /// 隣接する陸上国境を越えて弱い敵国へ実際に正当化(War Justification)を
    /// 開始することを、本番の公開エントリーポイント経由で検証する。
    #[test]
    fn test_war_preparation_ai_starts_justification_via_optimized_path() {
        let player_country = PlayerCountry(None);

        let strong = CountryData {
            id: CountryId(0),
            name: "Strong".to_string(),
            capital_state_id: StateId(0),
            treasury: 1000.0,
            ..default()
        };
        let weak = CountryData {
            id: CountryId(1),
            name: "Weak".to_string(),
            capital_state_id: StateId(1),
            treasury: 1000.0,
            ..default()
        };
        let mut country_registry = CountryRegistry {
            countries: vec![strong, weak],
        };

        let s0 = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(1)],
            ..default()
        };
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(0)],
            ..default()
        };
        let state_registry = StateRegistry::build(vec![s0, s1]);

        let mut military_registry = MilitaryRegistry::default();
        // Strong(0) は Weak(1) に対し130%以上の戦力優位を持つ
        military_registry.add_division(make_test_division(CountryId(0), StateId(0), 10_000));
        military_registry.add_division(make_test_division(CountryId(1), StateId(1), 1_000));

        let building_registry = crate::building::data::BuildingRegistry::default();
        let mut war_registry = WarRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        let tech_registry = TechnologyRegistry::default();
        let world_state = WorldCivilizationState::default();
        let mut justification_registry = WarJustificationRegistry::default();
        let mut frontline_registry = crate::war::frontline::FrontlineRegistry::default();
        let mut military_ai_registry = MilitaryAiRegistry::default();
        let mut country_ai_registry = CountryAiRegistry::default();

        // day=7: 週次評価 (day % 7 == country_id % 7) を Strong(0) に対して成立させる
        let mut pending_ai_wars = Vec::new();
        let mut crisis_registry = CrisisRegistry::default();
        process_daily_country_ai(
            7,
            "1800/01/08",
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut pending_ai_wars,
            &mut crisis_registry,
        );

        assert!(
            !justification_registry.justifications.is_empty(),
            "戦力優位かつ到達可能な隣国が存在する場合、最適化後の経路でも正当化が開始されること"
        );
        let j = justification_registry
            .justifications
            .values()
            .find(|j| j.initiator == CountryId(0))
            .expect("Strong(0)が開始した正当化が存在すること");
        assert_eq!(j.target, CountryId(1), "正当化の対象がWeak(1)であること");
        assert_eq!(
            j.target_state,
            StateId(1),
            "正当化の対象StateがState(1)であること"
        );

        // 注: `ai_state.dirty` は初期値 true のため、同日内で月次建設AI
        // (`process_construction_ai`) も実行され、`decision_reason` は
        // その建設判断理由へ後から上書きされる(本番の既存仕様)。
        // そのため war-prep AI が実際に機能したことは `mode` と
        // 正当化(justification)の内容で検証する。
        let ai_state = country_ai_registry.ai_states.get(&CountryId(0)).unwrap();
        assert_eq!(ai_state.mode, CountryAiMode::PreparingWar);
    }

    /// P21-017(二次レビュー対応・§27指摘2への対応): 自国開始の未解決Crisisが
    /// 進行中のAIは、`process_war_preparation_ai`が(戦力優位・到達可能な隣国という
    /// 条件を満たしていても)通常の正当化を独立に開始しない。上の
    /// `test_war_preparation_ai_starts_justification_via_optimized_path`と全く
    /// 同じ環境(Strong(0)がWeak(1)に対し130%以上の戦力優位、隣接)を使い、
    /// 唯一の違いはStrong(0)自身が開始した未解決Crisis(DemandSent)を事前に
    /// 挿入している点 — 以前はこのガードが無かったため、同じ対象国/州へ
    /// Crisis経由の正当化と生の正当化が並行して進行し得た(例: Crisisが
    /// 平和的に受諾された後も、並行して開始された生正当化だけが生き残り
    /// Readyになって不要な宣戦が起きる)。
    #[test]
    fn process_war_preparation_ai_does_not_start_a_competing_justification_while_a_crisis_is_unresolved()
     {
        let player_country = PlayerCountry(None);

        let strong = CountryData {
            id: CountryId(0),
            name: "Strong".to_string(),
            capital_state_id: StateId(0),
            treasury: 1000.0,
            ..default()
        };
        let weak = CountryData {
            id: CountryId(1),
            name: "Weak".to_string(),
            capital_state_id: StateId(1),
            treasury: 1000.0,
            ..default()
        };
        let mut country_registry = CountryRegistry {
            countries: vec![strong, weak],
        };

        let s0 = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(1)],
            ..default()
        };
        let s1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(0)],
            ..default()
        };
        let state_registry = StateRegistry::build(vec![s0, s1]);

        let mut military_registry = MilitaryRegistry::default();
        military_registry.add_division(make_test_division(CountryId(0), StateId(0), 10_000));
        military_registry.add_division(make_test_division(CountryId(1), StateId(1), 1_000));

        let building_registry = crate::building::data::BuildingRegistry::default();
        let mut war_registry = WarRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        let tech_registry = TechnologyRegistry::default();
        let world_state = WorldCivilizationState::default();
        let mut justification_registry = WarJustificationRegistry::default();
        let mut frontline_registry = crate::war::frontline::FrontlineRegistry::default();
        let mut military_ai_registry = MilitaryAiRegistry::default();
        let mut country_ai_registry = CountryAiRegistry::default();
        let mut pending_ai_wars = Vec::new();

        // Strong(0)自身が開始した、まだ解決していないCrisis(全く別の対象・州に
        // 対するものでもよい — このテストでは同じClaim由来かどうかを問わず、
        // 「未解決の自国開始Crisisがある」というだけで生の正当化探索が
        // 止まることを確認する)。
        let mut crisis_registry = CrisisRegistry::default();
        crisis_registry.add_crisis(crate::diplomacy::crisis::DiplomaticCrisis {
            id: crate::common::DiplomaticCrisisId(0),
            initiator: CountryId(0),
            target: CountryId(1),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: crate::diplomacy::crisis::CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: HashMap::new(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        });

        process_daily_country_ai(
            7,
            "1800/01/08",
            &player_country,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut pending_ai_wars,
            &mut crisis_registry,
        );

        assert!(
            justification_registry.justifications.is_empty(),
            "未解決の自国開始Crisisがある間は、戦力優位・到達可能な隣国が\
             存在してもprocess_war_preparation_aiは競合する正当化を開始しないこと"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P26-008-C: IndustrialGoods不足検出とFactory事前開発
    // ─────────────────────────────────────────────────────────────────────

    fn p26_008_c_building_registry() -> crate::building::data::BuildingRegistry {
        let mut reg = crate::building::data::BuildingRegistry::default();
        reg.definitions.insert(
            BuildingType::Farm,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Farm,
                name: "Farm".to_string(),
                construction_cost: 100.0,
                required_progress: 10.0,
                required_workforce: 100.0,
                logistics_cost: 0.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 1.0,
                max_level: 5,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        reg.definitions.insert(
            BuildingType::Mine,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Mine,
                name: "Mine".to_string(),
                construction_cost: 500.0,
                required_progress: 60.0,
                required_workforce: 10_000.0,
                logistics_cost: 10.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 15.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        reg.definitions.insert(
            BuildingType::Factory,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Factory,
                name: "Civilian Factory".to_string(),
                construction_cost: 1000.0,
                required_progress: 90.0,
                required_workforce: 20_000.0,
                logistics_cost: 15.0,
                input_resources: [(ResourceType::Iron, 20.0), (ResourceType::Coal, 20.0)]
                    .into_iter()
                    .collect(),
                output_resources: [(ResourceType::IndustrialGoods, 30.0)].into_iter().collect(),
                maintenance_cost: 30.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        reg.definitions.insert(
            BuildingType::MilitaryFactory,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::MilitaryFactory,
                name: "Military Factory".to_string(),
                construction_cost: 1200.0,
                required_progress: 90.0,
                required_workforce: 25_000.0,
                logistics_cost: 20.0,
                input_resources: [(ResourceType::Iron, 30.0), (ResourceType::IndustrialGoods, 15.0)]
                    .into_iter()
                    .collect(),
                output_resources: [
                    (ResourceType::MilitaryEquipment, 20.0),
                    (ResourceType::MilitarySupplies, 100.0),
                ]
                .into_iter()
                .collect(),
                maintenance_cost: 40.0,
                max_level: 10,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 0.0,
            },
        );
        reg.definitions.insert(
            BuildingType::Railway,
            crate::building::data::BuildingDefinition {
                building_type: BuildingType::Railway,
                name: "Railway".to_string(),
                construction_cost: 800.0,
                required_progress: 60.0,
                required_workforce: 3000.0,
                logistics_cost: 0.0,
                input_resources: HashMap::new(),
                output_resources: HashMap::new(),
                maintenance_cost: 20.0,
                max_level: 5,
                science_output: 0.0,
                magic_output: 0.0,
                railway_capacity_bonus: 50.0,
            },
        );
        reg
    }

    /// 優先度1(Food緊急)・優先度3の鉱床資源トリガー(Wood/Iron/Coal<30)のいずれも
    /// 発火しない、十分な備蓄を持つ国家(P26-008-Cのテストがpriority 2の
    /// IndustrialGoods分岐だけを単離して検証できるようにする)。
    fn p26_008_c_ample_country(id: CountryId, capital: StateId, treasury: f64) -> CountryData {
        let mut c = CountryData { id, capital_state_id: capital, treasury, ..CountryData::default() };
        c.stockpile.set(ResourceType::Food, 1000.0);
        c.stockpile.set(ResourceType::Wood, 1000.0);
        c.stockpile.set(ResourceType::Iron, 1000.0);
        c.stockpile.set(ResourceType::Coal, 1000.0);
        c
    }

    fn p26_008_c_owned_state(id: usize, owner: CountryId) -> StateData {
        StateData {
            id: StateId(id),
            name: format!("State {id}"),
            owner_country_id: owner,
            world_position: [id as f32 * 100.0, 0.0],
            size: [100.0, 100.0],
            ..StateData::default()
        }
    }

    fn p26_008_c_deposit(resource: ResourceType, discovered: bool) -> crate::economy::resources::StateResourceDeposit {
        crate::economy::resources::StateResourceDeposit {
            resource_type: resource,
            base_output: 10.0,
            discovered,
            development_level: 0,
        }
    }

    #[test]
    fn industrial_goods_gap_is_detected_when_a_militaryfactory_needs_it_and_no_factory_exists() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mine_state = p26_008_c_owned_state(2, country_id);
        mine_state.resource_deposits = vec![p26_008_c_deposit(ResourceType::Coal, true)];
        let mut mf_state = p26_008_c_owned_state(3, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);

        let state_registry = StateRegistry::build(vec![capital, mine_state, mf_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(
            country_id,
            &mut country_registry,
            &state_registry,
            &building_registry,
            &mut ai_state,
        );

        let country = country_registry.get(country_id).unwrap();
        assert_eq!(country.construction_queue.len(), 1);
        assert_eq!(country.construction_queue[0].building_type, BuildingType::Factory);
        assert_eq!(
            ai_state.decision_reason,
            CountryAiDecisionReason::SteelShortageSteelMillPriority
        );
    }

    #[test]
    fn no_industrial_goods_requiring_building_means_no_gap_is_detected() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        // MilitaryFactoryなし、Farmだけ所有(IndustrialGoodsを要求しない)。
        let mut farm_state = p26_008_c_owned_state(2, country_id);
        farm_state.buildings.insert(BuildingType::Farm, 1);

        let state_registry = StateRegistry::build(vec![capital, farm_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(country_id).unwrap();
        assert_ne!(
            ai_state.decision_reason,
            CountryAiDecisionReason::SteelShortageSteelMillPriority,
            "IndustrialGoodsを要求する所有建物がなければFactory優先分岐に入らないはず"
        );
        // 通常判断(優先度3)ではFactoryが選ばれ得るが、それはIndustrialGoods不足
        // 検出とは無関係な既存経路であることを理由コードで確認済み。
        let _ = country;
    }

    #[test]
    fn factory_level_one_means_the_structural_path_already_exists() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        let mut factory_state = p26_008_c_owned_state(3, country_id);
        factory_state.buildings.insert(BuildingType::Factory, 1);

        let state_registry = StateRegistry::build(vec![capital, mf_state, factory_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        assert_ne!(
            ai_state.decision_reason,
            CountryAiDecisionReason::SteelShortageSteelMillPriority,
            "Factory level>0が既に存在するなら、当月生産量に関わらずFactory優先を再発火しないはず"
        );
    }

    #[test]
    fn another_countrys_factory_does_not_satisfy_this_countrys_gap() {
        let self_id = CountryId(1);
        let other_id = CountryId(2);
        let capital = p26_008_c_owned_state(1, self_id);
        let mut mf_state = p26_008_c_owned_state(2, self_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        let mut mine_state = p26_008_c_owned_state(3, self_id);
        mine_state.resource_deposits = vec![p26_008_c_deposit(ResourceType::Coal, true)];
        // 他国の州にFactoryがあっても、自国の判定には影響しないはず。
        let mut other_factory_state = p26_008_c_owned_state(4, other_id);
        other_factory_state.buildings.insert(BuildingType::Factory, 1);

        let state_registry = StateRegistry::build(vec![capital, mf_state, mine_state, other_factory_state]);
        let mut country = p26_008_c_ample_country(self_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(self_id);

        process_construction_ai(self_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(self_id).unwrap();
        assert_eq!(
            country.construction_queue.len(),
            1,
            "他国のFactoryは自国のIndustrialGoods不足判定を満たさないはず"
        );
        assert_eq!(country.construction_queue[0].building_type, BuildingType::Factory);
    }

    #[test]
    fn food_emergency_takes_priority_over_the_industrial_goods_gap() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);

        let state_registry = StateRegistry::build(vec![capital, mf_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.stockpile.set(ResourceType::Food, 10.0); // 既存の緊急しきい値(50)未満
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::FoodShortageFarmPriority);
        let country = country_registry.get(country_id).unwrap();
        if let Some(item) = country.construction_queue.first() {
            assert_eq!(item.building_type, BuildingType::Farm);
        }
    }

    #[test]
    fn deposit_resource_priority_order_is_preserved_ahead_of_industrial_goods() {
        // MilitaryFactoryはIron・IndustrialGoodsの両方を要求する。ResourceType::ALLの
        // 宣言順(Food,Wood,Iron,Coal,MagicCrystal,IndustrialGoods,...)により、Iron鉱床の
        // 事前開発(Mine)がIndustrialGoods事前開発(Factory)より先に評価されるはず
        // (両方の資源が同時に不足している場合の既存tie-break、spec §9)。
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        let mut iron_state = p26_008_c_owned_state(3, country_id);
        iron_state.resource_deposits = vec![p26_008_c_deposit(ResourceType::Iron, true)];

        let state_registry = StateRegistry::build(vec![capital, mf_state, iron_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(country_id).unwrap();
        assert_eq!(country.construction_queue.len(), 1);
        assert_eq!(
            country.construction_queue[0].building_type,
            BuildingType::Mine,
            "Iron鉱床の事前開発はIndustrialGoods(Factory)より先に評価されるはず"
        );
    }

    #[test]
    fn factory_candidate_state_excludes_other_countries_and_invalid_states() {
        let self_id = CountryId(1);
        let other_id = CountryId(2);
        let capital = p26_008_c_owned_state(1, self_id);
        let mut mf_state = p26_008_c_owned_state(2, self_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        let foreign_state = p26_008_c_owned_state(3, other_id);

        let state_registry = StateRegistry::build(vec![capital, mf_state, foreign_state]);
        let mut country = p26_008_c_ample_country(self_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(self_id);

        process_construction_ai(self_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(self_id).unwrap();
        assert_eq!(country.construction_queue.len(), 1);
        let target_state = country.construction_queue[0].state_id;
        assert_ne!(target_state, StateId(3), "他国の州へ建設してはいけない");
        assert!(
            state_registry.get(target_state).is_some_and(|s| s.owner_country_id == self_id),
            "建設候補州は必ず自国州でなければならない"
        );
    }

    #[test]
    fn no_candidate_state_falls_through_without_panicking() {
        // 自国が所有する州がFactory候補として使える州(この国が所有する唯一の他州)
        // をIron鉱床のMineが既に埋めており、Factory自体は既存どおりcapitalへ
        // 建設可能なため、本来は候補が見つかる――ここでは意図的に州を1つ(capital)
        // だけにして、Mineの事前開発州がなくても既存の通常判断へ安全にフォール
        // スルーする(panicしない)ことを確認する。
        let country_id = CountryId(1);
        let mut capital = p26_008_c_owned_state(1, country_id);
        capital.buildings.insert(BuildingType::MilitaryFactory, 1);
        capital.buildings.insert(BuildingType::Factory, 10); // 既にmax levelまで建設済み

        let state_registry = StateRegistry::build(vec![capital]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        // パニックしないことそのものがこのテストの主張。
        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);
    }

    #[test]
    fn insufficient_funds_for_factory_falls_through_to_priority_three() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);

        let state_registry = StateRegistry::build(vec![capital, mf_state]);
        // Factory建設費(1000)未満、Mine建設費(500)以上。
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 600.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        assert_ne!(
            ai_state.decision_reason,
            CountryAiDecisionReason::SteelShortageSteelMillPriority,
            "Factory建設費を賄えない場合はこの規則を発火させず、優先度3へフォールスルーするはず"
        );
    }

    #[test]
    fn duplicate_factory_is_not_queued_while_one_is_already_under_construction() {
        // 注意: 優先度3(既存の通常判断、無変更)は原材料が十分なら独立に
        // 「経済成長のためFactoryを建設する」という既存の一般判断を行い得る――
        // これは本タスクのIndustrialGoods事前開発規則(優先度2)とは無関係な、
        // 既存の正当なAI挙動であり「重複」ではない。本テストが検証したいのは
        // あくまで優先度2自身が、既にFactoryが建設中の間は同じ理由
        // (`SteelShortageSteelMillPriority`)で再発火しないことである。
        let self_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, self_id);
        let mut mf_state = p26_008_c_owned_state(2, self_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);

        let state_registry = StateRegistry::build(vec![capital, mf_state]);
        let mut country = p26_008_c_ample_country(self_id, StateId(1), 100_000.0);
        // 既にFactoryが(別の州で)建設中であることを模擬する。
        country.construction_queue.clear();
        country.construction_queue.push(crate::building::construction::ConstructionQueueItem {
            state_id: StateId(1),
            building_type: BuildingType::Factory,
            target_level: 1,
            progress: 0.0,
            required_progress: 90.0,
            paid_cost: 1000.0,
            status: crate::building::construction::ConstructionStatus::InQueue,
        });
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(self_id);

        process_construction_ai(self_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        assert_ne!(
            ai_state.decision_reason,
            CountryAiDecisionReason::SteelShortageSteelMillPriority,
            "既にFactoryが建設中(未完成)なら、IndustrialGoods事前開発規則(優先度2)は\
             再発火してはいけない"
        );
    }

    #[test]
    fn staged_repair_moves_from_factory_to_coal_mine_once_factory_exists() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        // Factoryは既に完成済み(level>0)、CoalとIronの鉱床が両方ある州を用意。
        let mut factory_state = p26_008_c_owned_state(3, country_id);
        factory_state.buildings.insert(BuildingType::Factory, 1);
        let mut coal_state = p26_008_c_owned_state(4, country_id);
        coal_state.resource_deposits = vec![p26_008_c_deposit(ResourceType::Coal, true)];

        let state_registry =
            StateRegistry::build(vec![capital, mf_state, factory_state, coal_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        // Coal事前開発規則を発火させるため、あえてCoal備蓄を閾値未満にはしない
        // (事前開発はP26-004-PRE-002どおり閾値到達前の予防的発火のため、備蓄量とは無関係)。
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(country_id).unwrap();
        assert_eq!(country.construction_queue.len(), 1);
        assert_eq!(
            country.construction_queue[0].building_type,
            BuildingType::Mine,
            "Factory完成後は、Factory自身が要求するCoalの鉱床事前開発(Mine)へ\
             既存規則が接続されるはず"
        );
        assert_eq!(country.construction_queue[0].state_id, StateId(4));
    }

    #[test]
    fn no_coal_deposit_does_not_fabricate_a_mine() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1);
        let mut factory_state = p26_008_c_owned_state(3, country_id);
        factory_state.buildings.insert(BuildingType::Factory, 1);
        // Coal鉱床を持つ州は一切ない。

        let state_registry = StateRegistry::build(vec![capital, mf_state, factory_state]);
        let mut country = p26_008_c_ample_country(country_id, StateId(1), 100_000.0);
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let country = country_registry.get(country_id).unwrap();
        // Coal鉱床が皆無なので事前開発Mineは候補にならず、既存どおり優先度3
        // (原材料は十分なのでFactory)へフォールスルーするはず。不正なMineは
        // 生成されない。
        for item in &country.construction_queue {
            assert_ne!(
                item.building_type,
                BuildingType::Mine,
                "Coal鉱床が皆無の州へMineを建設してはいけない"
            );
        }
    }

    // ─── P27-002-A: 優先度3への3か月経常支出予備判定 ─────────────────────

    /// 優先度1(Food緊急)・優先度2(事前開発)のいずれも発火せず、優先度3の
    /// Mine分岐(Wood/Iron/Coal<30)だけを発火させる、鉱床ありの国家。
    fn p27_002_a_priority3_mine_country(treasury: f64, monthly_expenses: f64) -> (CountryData, StateData) {
        let mut country = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            treasury,
            monthly_expenses,
            ..CountryData::default()
        };
        country.stockpile.set(Res::Food, 100.0); // 優先度1を回避
        country.stockpile.set(Res::Wood, 0.0);
        country.stockpile.set(Res::Iron, 0.0);
        country.stockpile.set(Res::Coal, 0.0); // 優先度3のMine分岐を発火
        let state = iron_coal_deposit_state(StateId(1), CountryId(1));
        (country, state)
    }

    fn p27_002_a_registry_with_mine_def() -> crate::building::data::BuildingRegistry {
        let mut reg = crate::building::data::BuildingRegistry::default();
        reg.definitions.insert(BuildingType::Mine, mine_def());
        reg
    }

    #[test]
    fn p27_002_a_boundary_exact_allows_priority_3_construction() {
        let monthly_expenses = 100.0;
        let construction_cost = mine_def().construction_cost;
        let required_treasury = construction_cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
        let (country, state) = p27_002_a_priority3_mine_country(required_treasury, monthly_expenses);
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(vec![state]);
        let building_registry = p27_002_a_registry_with_mine_def();
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c.construction_queue.len(), 1, "境界ちょうどでは建設が成立するはず");
        assert_eq!(c.construction_queue[0].building_type, BuildingType::Mine);
        assert_eq!(
            c.treasury,
            monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS,
            "建設後、国庫は経常支出3か月分ちょうどまで減るはず"
        );
    }

    #[test]
    fn p27_002_a_boundary_under_defers_priority_3_construction_without_mutation() {
        let monthly_expenses = 100.0;
        let construction_cost = mine_def().construction_cost;
        let required_treasury = construction_cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;
        let treasury_before = required_treasury - 0.01;
        let (country, state) = p27_002_a_priority3_mine_country(treasury_before, monthly_expenses);
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(vec![state]);
        let building_registry = p27_002_a_registry_with_mine_def();
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert!(c.construction_queue.is_empty(), "境界未満では優先度3建設を見送るはず");
        assert_eq!(c.treasury, treasury_before, "見送り時は国庫を一切変更しないはず");
        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::FundsShortage);
    }

    #[test]
    fn p27_002_a_cost_itself_unaffordable_is_unchanged_from_existing_behavior() {
        let construction_cost = mine_def().construction_cost;
        let treasury_before = construction_cost - 1.0; // 建設費そのものにも満たない
        let (country, state) = p27_002_a_priority3_mine_country(treasury_before, 0.0);
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(vec![state]);
        let building_registry = p27_002_a_registry_with_mine_def();
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert!(c.construction_queue.is_empty());
        assert_eq!(c.treasury, treasury_before);
        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::FundsShortage);
    }

    #[test]
    fn p27_002_a_zero_recurring_expenses_matches_pre_existing_cost_only_boundary() {
        let construction_cost = mine_def().construction_cost;
        // monthly_expenses=0.0(New Game初月の未初期化値も含む)では、予備要件は
        // 常に0となり、既存(P27-002-A以前)のcost-onlyの境界と完全に一致するはず。
        let (country, state) = p27_002_a_priority3_mine_country(construction_cost, 0.0);
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(vec![state]);
        let building_registry = p27_002_a_registry_with_mine_def();
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c.construction_queue.len(), 1, "経常支出0なら従来どおりcostちょうどで建設できるはず");
        assert_eq!(c.treasury, 0.0);
    }

    #[test]
    fn p27_002_a_food_emergency_bypasses_the_reserve_gate() {
        let mut country = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            treasury: 150.0, // Farm(100)は払えるが、3か月予備(100+1000*3=3100)には遠く及ばない
            monthly_expenses: 1000.0,
            ..CountryData::default()
        };
        country.stockpile.set(Res::Food, 10.0); // <50、優先度1発火
        let state = no_deposit_state(StateId(1), CountryId(1));
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(vec![state]);
        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry.definitions.insert(BuildingType::Farm, {
            let mut def = mine_def();
            def.building_type = BuildingType::Farm;
            def.construction_cost = 100.0;
            def
        });
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c.construction_queue.len(), 1, "Food緊急は予備資金判定の対象外のはず");
        assert_eq!(c.construction_queue[0].building_type, BuildingType::Farm);
        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::FoodShortageFarmPriority);
    }

    #[test]
    fn p27_002_a_proactive_mine_development_bypasses_the_reserve_gate() {
        let mut c1 = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            treasury: 500.0, // Mine(500)は払えるが、3か月予備(500+1000*3=3500)には遠く及ばない
            monthly_expenses: 1000.0,
            ..CountryData::default()
        };
        healthy_stockpile(&mut c1); // 既存閾値(30)は上回っている
        let mut s1 = iron_coal_deposit_state(StateId(1), CountryId(1));
        s1.buildings.insert(BuildingType::MilitaryFactory, 1); // Iron需要はあるが稼働中Mineなし
        let state_registry = StateRegistry::build(vec![s1]);

        let mut building_registry = crate::building::data::BuildingRegistry::default();
        building_registry.definitions.insert(BuildingType::Mine, mine_def());
        building_registry.definitions.insert(BuildingType::Factory, factory_def());
        building_registry.definitions.insert(BuildingType::MilitaryFactory, military_factory_def());

        let mut country_registry = CountryRegistry { countries: vec![c1] };
        let mut ai_state = CountryAiState::new(CountryId(1));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(CountryId(1)).unwrap();
        assert_eq!(c.construction_queue.len(), 1, "供給網修復のMine事前開発は予備資金判定の対象外のはず");
        assert_eq!(c.construction_queue[0].building_type, BuildingType::Mine);
        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::RawMaterialMinePriority);
    }

    #[test]
    fn p27_002_a_proactive_factory_development_bypasses_the_reserve_gate() {
        let country_id = CountryId(1);
        let capital = p26_008_c_owned_state(1, country_id);
        let mut mf_state = p26_008_c_owned_state(2, country_id);
        mf_state.buildings.insert(BuildingType::MilitaryFactory, 1); // IndustrialGoods需要はあるがFactoryなし
        let state_registry = StateRegistry::build(vec![capital, mf_state]);

        let mut country = p26_008_c_ample_country(country_id, StateId(1), factory_def().construction_cost);
        // Factory(1000)は払えるが、3か月予備(1000+1000*3=4000)には遠く及ばない。
        country.monthly_expenses = 1000.0;
        country.construction_queue.clear();
        let mut country_registry = CountryRegistry { countries: vec![country] };
        let building_registry = p26_008_c_building_registry();
        let mut ai_state = CountryAiState::new(country_id);

        process_construction_ai(country_id, &mut country_registry, &state_registry, &building_registry, &mut ai_state);

        let c = country_registry.get(country_id).unwrap();
        assert_eq!(c.construction_queue.len(), 1, "供給網修復のFactory事前開発は予備資金判定の対象外のはず");
        assert_eq!(c.construction_queue[0].building_type, BuildingType::Factory);
        assert_eq!(ai_state.decision_reason, CountryAiDecisionReason::SteelShortageSteelMillPriority);
    }

    #[test]
    fn p27_002_a_rejects_non_finite_or_negative_financial_values_without_mutation() {
        let construction_cost = mine_def().construction_cost;
        let scenarios: [(&str, f64, f64); 4] = [
            ("treasury NaN", f64::NAN, 100.0),
            ("treasury Infinity", f64::INFINITY, 100.0),
            ("expenses NaN", construction_cost + 1000.0, f64::NAN),
            ("expenses negative", construction_cost + 1000.0, -50.0),
        ];
        for (label, treasury, monthly_expenses) in scenarios {
            let (country, state) = p27_002_a_priority3_mine_country(treasury, monthly_expenses);
            let mut country_registry = CountryRegistry { countries: vec![country] };
            let state_registry = StateRegistry::build(vec![state]);
            let building_registry = p27_002_a_registry_with_mine_def();
            let mut ai_state = CountryAiState::new(CountryId(1));

            process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state);

            let c = country_registry.get(CountryId(1)).unwrap();
            assert!(
                c.construction_queue.is_empty(),
                "[{label}] 非有限・負の財政値は安全側(見送り)へ倒すはず、mutationなし"
            );
            assert_eq!(
                c.treasury.to_bits(),
                treasury.to_bits(),
                "[{label}] 国庫はビット単位で不変のはず(NaN比較の罠を避けるためto_bits比較)"
            );
        }
    }

    #[test]
    fn p27_002_a_country_isolation_reserve_gate_does_not_cross_contaminate() {
        let monthly_expenses = 100.0;
        let construction_cost = mine_def().construction_cost;
        let required_treasury = construction_cost + monthly_expenses * PEACETIME_CONSTRUCTION_RESERVE_MONTHS;

        let (mut country_a, mut state_a) = p27_002_a_priority3_mine_country(required_treasury, monthly_expenses);
        country_a.id = CountryId(1);
        state_a.owner_country_id = CountryId(1);
        state_a.resource_deposits.iter_mut().for_each(|_| {});

        let (mut country_b, mut state_b) =
            p27_002_a_priority3_mine_country(required_treasury - 0.01, monthly_expenses);
        country_b.id = CountryId(2);
        state_b.id = StateId(2);
        state_b.owner_country_id = CountryId(2);

        let mut country_registry = CountryRegistry { countries: vec![country_a, country_b] };
        let state_registry = StateRegistry::build(vec![state_a, state_b]);
        let building_registry = p27_002_a_registry_with_mine_def();
        let mut ai_state_a = CountryAiState::new(CountryId(1));
        let mut ai_state_b = CountryAiState::new(CountryId(2));

        process_construction_ai(CountryId(1), &mut country_registry, &state_registry, &building_registry, &mut ai_state_a);
        process_construction_ai(CountryId(2), &mut country_registry, &state_registry, &building_registry, &mut ai_state_b);

        let a = country_registry.get(CountryId(1)).unwrap();
        let b = country_registry.get(CountryId(2)).unwrap();
        assert_eq!(a.construction_queue.len(), 1, "境界ちょうどの国家は建設が成立するはず");
        assert!(b.construction_queue.is_empty(), "境界未満の国家は見送るはず、他国の結果に影響されない");
        assert_eq!(b.treasury, required_treasury - 0.01, "他国の判定は自国のtreasuryへ影響しないはず");
    }

    // ─── P27-002-C: 行政費較正 + 予測安全なAI税率引下げ ──────────────────────

    fn tax_ai_country(id: usize, tax_rate: f32, monthly_balance: f64, monthly_expenses: f64) -> CountryData {
        CountryData { id: CountryId(id), tax_rate, monthly_balance, monthly_expenses, ..CountryData::default() }
    }

    fn tax_ai_state(state_id: usize, country_id: usize, population: u64, living_standard: f32) -> StateData {
        StateData {
            id: StateId(state_id),
            owner_country_id: CountryId(country_id),
            population,
            living_standard,
            ..StateData::default()
        }
    }

    /// 赤字AI国家は正規刻み(+0.01)で1回だけ引き上げられる(予測に関わらず)。
    #[test]
    fn p27_002_c_deficit_ai_country_raises_tax_rate_by_one_step() {
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.15, -10.0, 1_000_000.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        let c = country_registry.get(CountryId(1)).unwrap();
        assert!((c.tax_rate - 0.16).abs() < 1e-6, "赤字国は+0.01で引き上げられるはず: {}", c.tax_rate);
    }

    /// 黒字かつ1段階引下げ後の予測税収が経常支出を上回るAI国家は、0.15未満へも
    /// 安全に引き下げられる(P27-002-Cで0.15の絶対下限を撤廃)。
    #[test]
    fn p27_002_c_surplus_ai_country_with_safe_projection_decreases_below_0_15() {
        // pop=1,000,000 ls=50.0 candidate=0.14 → projected=700.0 >= expenses=600.0
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.15, 50.0, 600.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        let c = country_registry.get(CountryId(1)).unwrap();
        assert!((c.tax_rate - 0.14).abs() < 1e-6, "0.15未満へ安全に引き下げられるはず: {}", c.tax_rate);
    }

    /// 黒字でも予測税収が経常支出を下回るなら据え置く。
    #[test]
    fn p27_002_c_surplus_ai_country_with_unsafe_projection_holds() {
        // candidate=0.19 → projected=950.0 < expenses=10000.0
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.20, 50.0, 10_000.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        assert_eq!(country_registry.get(CountryId(1)).unwrap().tax_rate, 0.20, "不安全な引下げは据置くはず");
    }

    /// 月次収支ちょうど0は据え置く。
    #[test]
    fn p27_002_c_zero_monthly_balance_holds_tax_rate() {
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.20, 0.0, 100.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        assert_eq!(country_registry.get(CountryId(1)).unwrap().tax_rate, 0.20, "収支ちょうど0では据え置くはず");
    }

    /// 既存の税率上限(1.0)を超えない。
    #[test]
    fn p27_002_c_deficit_ai_country_clamps_at_the_existing_upper_bound() {
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.995, -10.0, 100.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        assert_eq!(country_registry.get(CountryId(1)).unwrap().tax_rate, 1.0);
    }

    /// `process_tax_rate_ai`はtax_rate以外のフィールド(当月の確定済み財政結果・
    /// 国庫)を一切変更しない——税率変更は次回の月次財政計算からしか反映されない、
    /// という非遡及性を直接保証する。
    #[test]
    fn p27_002_c_tax_rate_change_does_not_retroactively_alter_the_already_finalized_month() {
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.15, -10.0, 1_000_000.0)] };
        country_registry.get_mut(CountryId(1)).unwrap().monthly_income = 123.0;
        country_registry.get_mut(CountryId(1)).unwrap().treasury = 999.0;
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, 50.0)]);
        let before = country_registry.get(CountryId(1)).unwrap().clone();

        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);

        let after = country_registry.get(CountryId(1)).unwrap();
        assert_ne!(after.tax_rate, before.tax_rate, "tax_rateは変更されるはず");
        assert_eq!(after.monthly_income, before.monthly_income);
        assert_eq!(after.monthly_expenses, before.monthly_expenses);
        assert_eq!(after.monthly_balance, before.monthly_balance);
        assert_eq!(after.treasury, before.treasury, "当月の確定済み国庫を遡及して変更してはいけない");
    }

    /// 非有限な州のliving_standardは既存validationにより安全に拒否され、税率を
    /// 破壊しない(0コアーションではなく据置)。
    #[test]
    fn p27_002_c_rejects_non_finite_state_living_standard_without_mutation() {
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.20, 50.0, 1.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 1, 1_000_000, f32::NAN)]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        assert_eq!(country_registry.get(CountryId(1)).unwrap().tax_rate, 0.20);
    }

    /// 一国の人口・生活水準・経常支出が、他国の税率判断へ混入しない。
    #[test]
    fn p27_002_c_country_isolation_population_and_living_standard_do_not_cross_contaminate() {
        let mut country_registry = CountryRegistry {
            countries: vec![
                tax_ai_country(1, 0.15, -10.0, 1_000_000.0), // 赤字 → 引き上げ
                tax_ai_country(2, 0.20, 50.0, 10_000.0),     // 黒字だが不安全 → 据置
            ],
        };
        let state_registry = StateRegistry::build(vec![
            tax_ai_state(1, 1, 1_000_000, 50.0),
            tax_ai_state(2, 2, 1_000_000, 50.0),
        ]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        process_tax_rate_ai(CountryId(2), &mut country_registry, &state_registry);
        assert!((country_registry.get(CountryId(1)).unwrap().tax_rate - 0.16).abs() < 1e-6);
        assert_eq!(
            country_registry.get(CountryId(2)).unwrap().tax_rate,
            0.20,
            "他国の赤字判定・州データが混入してはいけない"
        );
    }

    /// 平均生活水準は人口加重平均でなければならない——単純平均を使うと、人口の
    /// 大きい州の実態を過小評価し、本会計の税収と乖離してしまう。
    #[test]
    fn p27_002_c_average_living_standard_used_for_projection_is_population_weighted() {
        // 州1: 人口9,000,000・ls=10.0(大多数)。州2: 人口1,000,000・ls=90.0(少数)。
        // 単純平均は(10+90)/2=50.0だが、人口加重平均は
        // (9,000,000*10+1,000,000*90)/10,000,000=18.0。
        // candidate=0.19のとき:
        //   人口加重平均(18.0)での予測 = 10,000,000*18.0*0.19*0.0001 = 3420.0
        //   単純平均(50.0)での予測(誤り)  = 10,000,000*50.0*0.19*0.0001 = 9500.0
        // expenses=5000.0はこの2値の間にあるため、正しい実装(加重平均)なら
        // 3420.0 < 5000.0で据置き、誤って単純平均を使えば9500.0 >= 5000.0で
        // 引下げてしまう——両者を明確に区別できる境界値である。
        let mut country_registry = CountryRegistry { countries: vec![tax_ai_country(1, 0.20, 50.0, 5000.0)] };
        let state_registry = StateRegistry::build(vec![
            tax_ai_state(1, 1, 9_000_000, 10.0),
            tax_ai_state(2, 1, 1_000_000, 90.0),
        ]);
        process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
        assert_eq!(
            country_registry.get(CountryId(1)).unwrap().tax_rate,
            0.20,
            "人口加重平均(18.0→予測3420.0<5000.0)なら不安全と正しく判定され据置くはず \
             (単純平均50.0→予測9500.0だと誤って引下げてしまう)"
        );
    }

    /// 国家登録順・州登録順を反転しても結果が変わらない。
    #[test]
    fn p27_002_c_result_does_not_depend_on_country_or_state_registration_order() {
        fn run(reverse: bool) -> f32 {
            let mut countries =
                vec![tax_ai_country(1, 0.15, 50.0, 600.0), tax_ai_country(2, 0.20, -10.0, 100.0)];
            let mut states =
                vec![tax_ai_state(1, 1, 1_000_000, 50.0), tax_ai_state(2, 2, 1_000_000, 50.0)];
            if reverse {
                countries.reverse();
                states.reverse();
            }
            let mut country_registry = CountryRegistry { countries };
            let state_registry = StateRegistry::build(states);
            process_tax_rate_ai(CountryId(1), &mut country_registry, &state_registry);
            country_registry.get(CountryId(1)).unwrap().tax_rate
        }
        assert_eq!(run(false), run(true));
    }

    /// `process_daily_country_ai`経由(プレイヤー/消滅国家フィルタ・cadenceを含む
    /// 実際の日次AIループ)でのテスト用最小ヘルパー。
    #[allow(clippy::type_complexity)]
    fn run_daily_country_ai_minimal(
        current_day: u32,
        player_country: &PlayerCountry,
        country_registry: &mut CountryRegistry,
        state_registry: &StateRegistry,
        country_ai_registry: &mut CountryAiRegistry,
    ) {
        let building_registry = crate::building::data::BuildingRegistry::default();
        let military_registry = MilitaryRegistry::default();
        let mut war_registry = WarRegistry::default();
        let mut diplomacy_registry = DiplomacyRegistry::default();
        let tech_registry = TechnologyRegistry::default();
        let world_state = WorldCivilizationState::default();
        let mut justification_registry = WarJustificationRegistry::default();
        let mut frontline_registry = crate::war::frontline::FrontlineRegistry::default();
        let mut military_ai_registry = MilitaryAiRegistry::default();
        let mut pending_ai_wars = Vec::new();
        let mut crisis_registry = CrisisRegistry::default();
        process_daily_country_ai(
            current_day,
            "1800/01/01",
            player_country,
            country_registry,
            state_registry,
            &building_registry,
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            country_ai_registry,
            &mut pending_ai_wars,
            &mut crisis_registry,
        );
    }

    /// プレイヤー国家は同じ赤字状態でも税率を自動変更されない。同じ日に評価される
    /// 他のAI国家は正規どおり調整される(プレイヤー保護がAI全体を止めてしまわない
    /// ことも同時に確認する)。
    #[test]
    fn p27_002_c_player_country_tax_rate_is_never_auto_adjusted() {
        let player_country = PlayerCountry(Some(CountryId(1)));
        let mut country_registry = CountryRegistry {
            countries: vec![
                tax_ai_country(1, 0.15, -10.0, 1_000_000.0),
                tax_ai_country(2, 0.15, -10.0, 1_000_000.0),
            ],
        };
        let state_registry = StateRegistry::build(vec![
            tax_ai_state(1, 1, 1_000_000, 50.0),
            tax_ai_state(2, 2, 1_000_000, 50.0),
        ]);
        let mut country_ai_registry = CountryAiRegistry::default();
        run_daily_country_ai_minimal(1, &player_country, &mut country_registry, &state_registry, &mut country_ai_registry);
        assert_eq!(
            country_registry.get(CountryId(1)).unwrap().tax_rate,
            0.15,
            "プレイヤー国家の税率はAIが自動変更してはいけない"
        );
        assert!(
            (country_registry.get(CountryId(2)).unwrap().tax_rate - 0.16).abs() < 1e-6,
            "同じ赤字状態でもAI国家は正規どおり調整されるはず"
        );
    }

    /// 消滅国家(Eliminated)は税率を変更されない。
    #[test]
    fn p27_002_c_eliminated_country_tax_rate_is_never_adjusted() {
        let player_country = PlayerCountry(None);
        let mut eliminated = tax_ai_country(2, 0.15, -10.0, 1_000_000.0);
        eliminated.status = crate::country::CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".to_string(),
            annexed_by: CountryId(1),
        };
        let mut country_registry =
            CountryRegistry { countries: vec![tax_ai_country(1, 0.15, -10.0, 1_000_000.0), eliminated] };
        let state_registry = StateRegistry::build(vec![
            tax_ai_state(1, 1, 1_000_000, 50.0),
            tax_ai_state(2, 2, 1_000_000, 50.0),
        ]);
        let mut country_ai_registry = CountryAiRegistry::default();
        run_daily_country_ai_minimal(1, &player_country, &mut country_registry, &state_registry, &mut country_ai_registry);
        assert_eq!(
            country_registry.get(CountryId(2)).unwrap().tax_rate,
            0.15,
            "消滅国家の税率はAIが変更してはいけない"
        );
    }

    /// 同月中に複数回Systemが呼ばれても、正規cadence(月1回)以上には変更されない。
    /// New Game直後の`dirty=true`によりday1で初回評価が起こる点は既存の仕様
    /// (P26-008-PRE-002が復元)であり、day2(CountryId(2)の正規月次cadence
    /// `day%30==2`)での2回目の評価と合わせて、30日間で正確に2回(dirty初回+
    /// cadence1回)だけ評価されることを確認する。
    #[test]
    fn p27_002_c_tax_rate_changes_at_most_once_per_monthly_evaluation_cycle() {
        let player_country = PlayerCountry(None);
        let mut country_registry =
            CountryRegistry { countries: vec![tax_ai_country(2, 0.15, -10.0, 1_000_000.0)] };
        let state_registry = StateRegistry::build(vec![tax_ai_state(1, 2, 1_000_000, 50.0)]);
        let mut country_ai_registry = CountryAiRegistry::default();

        run_daily_country_ai_minimal(1, &player_country, &mut country_registry, &state_registry, &mut country_ai_registry);
        assert!((country_registry.get(CountryId(2)).unwrap().tax_rate - 0.16).abs() < 1e-6);

        for day in 2..=30u32 {
            run_daily_country_ai_minimal(day, &player_country, &mut country_registry, &state_registry, &mut country_ai_registry);
        }
        let after_month = country_registry.get(CountryId(2)).unwrap().tax_rate;
        assert!(
            (after_month - 0.17).abs() < 1e-6,
            "30日間でdirty初回(day1)+正規cadence(day2)の合計2回だけ変更されるはず: {after_month}"
        );
    }

    // ─── P28-002: 戦時MilitaryFactory建設(優先度2.5) ─────────────────────────

    /// 労働力50,000(人口100,000×0.5)の州。民間労働力全員を未雇用として置き、
    /// 月次雇用計算を経ずに増産見込みの式だけを検証できるようにする。
    fn p28_state(id: usize, owner: CountryId, logistics_capacity: f32) -> StateData {
        let mut s = p26_008_c_owned_state(id, owner);
        s.population = 100_000;
        s.workforce_ratio = 0.5;
        s.employed_workforce = 0;
        s.unemployed_workforce = 50_000;
        s.logistics_capacity = logistics_capacity;
        s.logistics_usage = 0.0;
        s
    }

    /// 建物を置き、物流使用量を建物定義どおりに合わせる(`update_state_logistics`と同じ式)。
    fn p28_with_building(mut s: StateData, b: BuildingType, level: u32, operation: f32) -> StateData {
        let reg = p26_008_c_building_registry();
        s.buildings.insert(b, level);
        s.building_operation_ratios.insert(b, operation);
        s.logistics_usage = s
            .buildings
            .iter()
            .map(|(bt, lv)| reg.get(*bt).map(|d| d.logistics_cost * *lv as f32).unwrap_or(0.0))
            .sum();
        s
    }

    fn p28_country(id: CountryId, capital: StateId, treasury: f64) -> CountryData {
        let mut c = p26_008_c_ample_country(id, capital, treasury);
        c.stockpile.set(ResourceType::IndustrialGoods, 1000.0);
        c.monthly_expenses = 100.0;
        c
    }

    /// 需要2.0/日(=月60)の戦時文脈。
    const P28_TWO_DIVISIONS: WartimeSupplyContext = WartimeSupplyContext { daily_demand_units: 2_000_000 };

    /// 基準シナリオ: 州1にFactory lv1(IndustrialGoodsの構造的経路があるので優先度2は
    /// 発火しない)とMilitaryFactory lv1(前月稼働率0.2 = 月産20 < 需要60)。
    fn p28_base_state(owner: CountryId) -> StateData {
        let s = p28_state(1, owner, 100.0);
        let s = p28_with_building(s, BuildingType::Factory, 1, 1.0);
        p28_with_building(s, BuildingType::MilitaryFactory, 1, 0.2)
    }

    fn p28_run(
        country: CountryData,
        states: Vec<StateData>,
        wartime: Option<WartimeSupplyContext>,
    ) -> (CountryData, CountryAiDecisionReason) {
        let id = country.id;
        let mut registry = CountryRegistry { countries: vec![country] };
        let state_registry = StateRegistry::build(states);
        let mut ai_state = CountryAiState::new(id);
        process_construction_ai_with_wartime_supply(
            id,
            &mut registry,
            &state_registry,
            &p26_008_c_building_registry(),
            &mut ai_state,
            wartime,
        );
        (registry.countries.remove(0), ai_state.decision_reason)
    }

    fn p28_queued_military_factories(country: &CountryData) -> Vec<(StateId, u32)> {
        country
            .construction_queue
            .iter()
            .filter(|q| q.building_type == BuildingType::MilitaryFactory)
            .map(|q| (q.state_id, q.target_level))
            .collect()
    }

    #[test]
    fn p28_projected_gain_uses_workforce_and_logistics_of_the_candidate_state() {
        let reg = p26_008_c_building_registry();
        let def = reg.get(BuildingType::MilitaryFactory).unwrap();
        // 州1: 総必要労働力45,000/労働力50,000 → 労働力比0.9、物流35/100 → 1.0。
        // 追加前 100×1×0.9 = 90、追加後 必要70,000 → 雇用50,000 → 1.0、物流55/100 → 1.0 → 200。
        let gain = projected_military_factory_supply_gain(&p28_base_state(CountryId(1)), def, &reg).unwrap();
        // 労働力比は本番と同じf32で計算するため、0.9のf32表現分の誤差を許容する。
        assert!((gain - 110.0).abs() < 1e-4, "gain={gain}");
        // 物流が飽和した州(容量35、使用35)。追加後は35/55で頭打ちになる。
        let mut tight = p28_base_state(CountryId(1));
        tight.logistics_capacity = 35.0;
        let tight_gain = projected_military_factory_supply_gain(&tight, def, &reg).unwrap();
        assert!((tight_gain - (200.0 * 35.0 / 55.0 - 90.0)).abs() < 1e-3, "tight_gain={tight_gain}");
        // 上限levelの州と労働力0の州は候補外。
        let maxed = p28_with_building(p28_state(2, CountryId(1), 1000.0), BuildingType::MilitaryFactory, 10, 1.0);
        assert_eq!(projected_military_factory_supply_gain(&maxed, def, &reg), None);
        let mut empty = p28_state(3, CountryId(1), 100.0);
        empty.population = 0;
        assert_eq!(projected_military_factory_supply_gain(&empty, def, &reg), None);
    }

    #[test]
    fn p28_wartime_ai_starts_exactly_one_military_factory_when_realised_supply_falls_short() {
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, reason) = p28_run(country, vec![p28_base_state(CountryId(1))], Some(P28_TWO_DIVISIONS));
        assert_eq!(p28_queued_military_factories(&after), vec![(StateId(1), 2)]);
        assert_eq!(after.construction_queue.len(), 1, "1回の判断で追加するのは1件だけ");
        assert!((after.treasury - 3800.0).abs() < 1e-9, "既存の支払い経路で建設費1200だけを支払う");
        assert_eq!(reason, CountryAiDecisionReason::WarInProgress);
    }

    #[test]
    fn p28_peacetime_path_never_starts_a_military_factory() {
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![p28_base_state(CountryId(1))], None);
        assert!(p28_queued_military_factories(&after).is_empty(), "戦時文脈なしでは新経路は評価されない");
    }

    #[test]
    fn p28_state_is_chosen_by_projected_supply_gain_not_by_level_or_state_id() {
        // 州1: StateIdが小さくMilitaryFactory levelも高いが、物流容量60がほぼ飽和(増産見込み≒12.6)。
        let s1 = p28_with_building(p28_state(1, CountryId(1), 60.0), BuildingType::Factory, 1, 1.0);
        let s1 = p28_with_building(s1, BuildingType::MilitaryFactory, 3, 0.1);
        // 州2: 建物なし、労働力比0.5・物流1.0(増産見込み50)。
        let s2 = p28_state(2, CountryId(1), 100.0);
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![s1, s2], Some(P28_TWO_DIVISIONS));
        assert_eq!(p28_queued_military_factories(&after), vec![(StateId(2), 1)]);
    }

    #[test]
    fn p28_equal_gain_breaks_ties_by_state_id_regardless_of_registration_order() {
        let mut s1 = p28_base_state(CountryId(1));
        s1.logistics_capacity = 35.0; // 増産見込み≒37 < 州2・州3の50
        let build = |states: Vec<StateData>| {
            let country = p28_country(CountryId(1), StateId(1), 5000.0);
            p28_queued_military_factories(&p28_run(country, states, Some(P28_TWO_DIVISIONS)).0)
        };
        let forward = build(vec![s1.clone(), p28_state(2, CountryId(1), 100.0), p28_state(3, CountryId(1), 100.0)]);
        let reversed = build(vec![p28_state(3, CountryId(1), 100.0), p28_state(2, CountryId(1), 100.0), s1]);
        assert_eq!(forward, vec![(StateId(2), 1)]);
        assert_eq!(reversed, forward);
    }

    #[test]
    fn p28_missing_inputs_block_the_wartime_factory() {
        // 追加後の月次需要: Iron = Factory 20 + MF 30 + 新規30 = 80、IndustrialGoods = 15 + 新規15 = 30。
        for (resource, short, exact) in [(ResourceType::Iron, 79.99, 80.0), (ResourceType::IndustrialGoods, 29.99, 30.0)] {
            let mut country = p28_country(CountryId(1), StateId(1), 5000.0);
            country.stockpile.set(resource, short);
            let (after, _) = p28_run(country, vec![p28_base_state(CountryId(1))], Some(P28_TWO_DIVISIONS));
            assert!(p28_queued_military_factories(&after).is_empty(), "{resource:?}が1か月分未満");
            let mut country = p28_country(CountryId(1), StateId(1), 5000.0);
            country.stockpile.set(resource, exact);
            let (after, _) = p28_run(country, vec![p28_base_state(CountryId(1))], Some(P28_TWO_DIVISIONS));
            assert_eq!(p28_queued_military_factories(&after).len(), 1, "{resource:?}がちょうど1か月分なら着工");
        }
    }

    #[test]
    fn p28_treasury_threshold_is_cost_plus_one_month_of_expenses() {
        // 必要額 = 1200 + 100 × 1 = 1300。
        let (below, _) = p28_run(
            p28_country(CountryId(1), StateId(1), 1299.99),
            vec![p28_base_state(CountryId(1))],
            Some(P28_TWO_DIVISIONS),
        );
        assert!(p28_queued_military_factories(&below).is_empty());
        assert!((below.treasury - 1299.99).abs() < 1e-9, "着工しない場合は国庫を変更しない");
        let (exact, _) = p28_run(
            p28_country(CountryId(1), StateId(1), 1300.0),
            vec![p28_base_state(CountryId(1))],
            Some(P28_TWO_DIVISIONS),
        );
        assert_eq!(p28_queued_military_factories(&exact), vec![(StateId(1), 2)]);
        assert!((exact.treasury - 100.0).abs() < 1e-9);
        // 非有限の経常支出は着工しない側に倒す。
        let mut nan = p28_country(CountryId(1), StateId(1), 5000.0);
        nan.monthly_expenses = f64::NAN;
        let (after, _) = p28_run(nan, vec![p28_base_state(CountryId(1))], Some(P28_TWO_DIVISIONS));
        assert!(p28_queued_military_factories(&after).is_empty());
    }

    #[test]
    fn p28_no_buildable_state_no_demand_or_sufficient_supply_means_no_factory() {
        // 建設可能州なし(上限level)。
        let maxed = p28_with_building(
            p28_with_building(p28_state(1, CountryId(1), 1000.0), BuildingType::Factory, 1, 1.0),
            BuildingType::MilitaryFactory,
            10,
            0.0,
        );
        let (after, _) =
            p28_run(p28_country(CountryId(1), StateId(1), 5000.0), vec![maxed], Some(P28_TWO_DIVISIONS));
        assert!(p28_queued_military_factories(&after).is_empty());
        // 需要0。
        let (after, _) = p28_run(
            p28_country(CountryId(1), StateId(1), 5000.0),
            vec![p28_base_state(CountryId(1))],
            Some(WartimeSupplyContext { daily_demand_units: 0 }),
        );
        assert!(p28_queued_military_factories(&after).is_empty());
        // 実月産が需要とちょうど等しい(稼働率0.6 → 60 = 60)。
        let mut enough = p28_base_state(CountryId(1));
        enough.building_operation_ratios.insert(BuildingType::MilitaryFactory, 0.6);
        let (after, _) =
            p28_run(p28_country(CountryId(1), StateId(1), 5000.0), vec![enough], Some(P28_TWO_DIVISIONS));
        assert!(p28_queued_military_factories(&after).is_empty());
    }

    #[test]
    fn p28_unfinished_military_factory_is_never_ordered_twice() {
        let country = p28_country(CountryId(1), StateId(1), 10_000.0);
        let mut registry = CountryRegistry { countries: vec![country] };
        let state_registry =
            StateRegistry::build(vec![p28_base_state(CountryId(1)), p28_state(2, CountryId(1), 100.0)]);
        let mut ai_state = CountryAiState::new(CountryId(1));
        let building_registry = p26_008_c_building_registry();
        for _ in 0..2 {
            process_construction_ai_with_wartime_supply(
                CountryId(1),
                &mut registry,
                &state_registry,
                &building_registry,
                &mut ai_state,
                Some(P28_TWO_DIVISIONS),
            );
        }
        assert_eq!(
            p28_queued_military_factories(&registry.countries[0]).len(),
            1,
            "キュー上の未完了MilitaryFactoryがある間は新規発注しない"
        );
    }

    #[test]
    fn p28_food_emergency_and_supply_chain_repair_keep_priority_over_the_wartime_factory() {
        // 優先度1: Food < 50。
        let mut hungry = p28_country(CountryId(1), StateId(1), 5000.0);
        hungry.stockpile.set(ResourceType::Food, 49.0);
        let (after, reason) = p28_run(hungry, vec![p28_base_state(CountryId(1))], Some(P28_TWO_DIVISIONS));
        assert_eq!(reason, CountryAiDecisionReason::FoodShortageFarmPriority);
        assert!(p28_queued_military_factories(&after).is_empty());
        // 優先度2: MilitaryFactoryがIndustrialGoodsを要求するのにFactoryがない。
        let no_factory = p28_with_building(p28_state(1, CountryId(1), 100.0), BuildingType::MilitaryFactory, 1, 0.2);
        let (after, reason) =
            p28_run(p28_country(CountryId(1), StateId(1), 5000.0), vec![no_factory], Some(P28_TWO_DIVISIONS));
        assert_eq!(reason, CountryAiDecisionReason::SteelShortageSteelMillPriority);
        assert!(p28_queued_military_factories(&after).is_empty());
        assert_eq!(after.construction_queue[0].building_type, BuildingType::Factory);
    }

    #[test]
    fn p28_wartime_one_month_reserve_does_not_leak_into_peacetime_development() {
        // 実月産が需要を満たす(優先度2.5は発火しない)とき、優先度3は従来どおり3か月予備を要求する:
        // Factory 1000 + 200 × 3 = 1600 > 国庫1500 → FundsShortage(1か月予備なら1200で足りる額)。
        let mut enough = p28_base_state(CountryId(1));
        enough.building_operation_ratios.insert(BuildingType::MilitaryFactory, 0.6);
        let mut country = p28_country(CountryId(1), StateId(1), 1500.0);
        country.monthly_expenses = 200.0;
        let (after, reason) = p28_run(country, vec![enough], Some(P28_TWO_DIVISIONS));
        assert!(after.construction_queue.is_empty());
        assert_eq!(reason, CountryAiDecisionReason::FundsShortage);
    }

    #[test]
    fn p28_daily_ai_applies_the_wartime_factory_only_to_active_ai_countries_at_war() {
        use crate::military::data::{DivisionDefinition, DivisionSize, DivisionType};
        let (
            player_country,
            _,
            _,
            _,
            _,
            _,
            mut diplomacy_registry,
            tech_registry,
            world_state,
            mut justification_registry,
            mut frontline_registry,
            mut military_ai_registry,
            mut country_ai_registry,
        ) = setup_test_env();
        // 1=プレイヤー(戦時)、2=AI(戦時)、3=消滅国家(戦争記録上は交戦中)、4=AI(平時)。
        let mut countries: Vec<CountryData> =
            (1..=4).map(|i| p28_country(CountryId(i), StateId(i), 10_000.0)).collect();
        countries[2].status = crate::country::CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".into(),
            annexed_by: CountryId(2),
        };
        let mut country_registry = CountryRegistry { countries };
        let states: Vec<StateData> = (1..=4)
            .map(|i| {
                let s = p28_with_building(p28_state(i, CountryId(i), 100.0), BuildingType::Factory, 1, 1.0);
                p28_with_building(s, BuildingType::MilitaryFactory, 1, 0.2)
            })
            .collect();
        let state_registry = StateRegistry::build(states);
        let mut military_registry = MilitaryRegistry::default();
        military_registry.definitions.insert(
            crate::common::DivisionDefinitionId(0),
            DivisionDefinition {
                id: crate::common::DivisionDefinitionId(0),
                name: "Infantry".into(),
                division_type: DivisionType::Infantry,
                size: DivisionSize::Standard,
                required_manpower: 10_000,
                required_equipment: 10.0,
                recruitment_days: 30,
                movement_speed: 1.0,
                attack: 10.0,
                defense: 10.0,
                breakthrough: 1.0,
                organization: 100.0,
                morale: 1.0,
                supply_usage: 1.0,
                maintenance_cost: 1.0,
            },
        );
        for (n, owner) in [1usize, 1, 2, 2, 3, 3, 4, 4].into_iter().enumerate() {
            let mut d = make_test_division(CountryId(owner), StateId(owner), 10_000);
            d.id = crate::common::DivisionId(n);
            military_registry.divisions.insert(d.id, d);
        }
        let mut war_registry = WarRegistry::default();
        for (attacker, defender) in [(2, 1), (3, 1)] {
            war_registry.add_war(crate::war::data::War {
                id: WarId(0),
                name: "test".into(),
                attackers: [CountryId(attacker)].into_iter().collect(),
                defenders: [CountryId(defender)].into_iter().collect(),
                primary_attacker: None,
                primary_defender: None,
                source_crisis_id: None,
                war_goals: vec![],
                start_date: "1800/01/01".into(),
                end_date: None,
                duration_days: 0,
                war_score: 0.0,
                attacker_war_exhaustion: 0.0,
                defender_war_exhaustion: 0.0,
                occupied_states: Default::default(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: vec![],
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: Default::default(),
            });
        }
        process_daily_country_ai(
            1,
            "1800/01/02",
            &player_country,
            &mut country_registry,
            &state_registry,
            &p26_008_c_building_registry(),
            &military_registry,
            &mut war_registry,
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut Vec::new(),
            &mut CrisisRegistry::default(),
        );
        let queued = |id: usize| p28_queued_military_factories(country_registry.get(CountryId(id)).unwrap());
        assert!(queued(1).is_empty(), "プレイヤー国家には適用しない");
        assert_eq!(queued(2), vec![(StateId(2), 2)], "戦時のAI国家だけが着工する");
        assert!(queued(3).is_empty(), "消滅国家には適用しない");
        assert!(queued(4).is_empty(), "平時のAI国家には適用しない");
    }

    // ─── P28-005: 物流生活水準維持のためのRailway優先建設(優先度2.7) ──────────

    fn p28_005_queued_railways(country: &CountryData) -> Vec<(StateId, u32)> {
        country
            .construction_queue
            .iter()
            .filter(|q| q.building_type == BuildingType::Railway)
            .map(|q| (q.state_id, q.target_level))
            .collect()
    }

    /// Food/Wood/Iron/Coalが潤沢で、Factory lv1(IndustrialGoodsの構造的経路が
    /// あるので優先度2は発火しない)を持つ州。`logistics_ratio`は呼び出し側が
    /// 個別に設定する。
    fn p28_005_state(id: usize, owner: CountryId, logistics_ratio: f32) -> StateData {
        let mut s = p28_with_building(p28_state(id, owner, 100.0), BuildingType::Factory, 1, 1.0);
        s.logistics_ratio = logistics_ratio;
        s
    }

    /// 契約1: 物流比率0.9未満の適格州があれば着工できる。
    #[test]
    fn p28_005_builds_railway_when_a_state_is_below_the_logistics_bonus_ratio() {
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, reason) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.5)], None);
        assert_eq!(p28_005_queued_railways(&after), vec![(StateId(1), 1)]);
        assert_eq!(after.construction_queue.len(), 1, "1回の判断で追加するのは1件だけ");
        assert!((after.treasury - 4200.0).abs() < 1e-9, "建設費800だけを支払う");
        assert_eq!(reason, CountryAiDecisionReason::PeacetimeDevelopment);
    }

    /// 契約2: 物流比率0.9以上の州だけなら、この段階では着工しない
    /// (0.9ちょうども含めて境界値を確認する)。
    #[test]
    fn p28_005_does_not_build_when_all_states_are_at_or_above_the_logistics_bonus_ratio() {
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.9)], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "0.9ちょうどは対象外");

        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 1.0)], None);
        assert!(p28_005_queued_railways(&after).is_empty());
    }

    /// 契約3: 雇用率が高くても、物流不足だけで対象から除外しない
    /// (P28-005-PRE-002のR1が境界依存の保護漏れを起こしたことの直接対照)。
    #[test]
    fn p28_005_high_employment_state_with_low_logistics_is_still_eligible() {
        let mut s = p28_005_state(1, CountryId(1), 0.5);
        s.employed_workforce = 50_000; // total_workforce=50,000 → 雇用率1.0
        s.unemployed_workforce = 0;
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![s], None);
        assert_eq!(
            p28_005_queued_railways(&after),
            vec![(StateId(1), 1)],
            "雇用率1.0でも物流比率0.5なら着工する(雇用率フィルタを追加しない)"
        );
    }

    /// 契約4: 複数候補では物流比率が最も低い州を選ぶ。
    #[test]
    fn p28_005_lowest_logistics_ratio_state_is_selected_among_multiple_candidates() {
        let s1 = p28_005_state(1, CountryId(1), 0.7);
        let s2 = p28_005_state(2, CountryId(1), 0.3);
        let s3 = p28_005_state(3, CountryId(1), 0.8);
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, _) = p28_run(country, vec![s1, s2, s3], None);
        assert_eq!(p28_005_queued_railways(&after), vec![(StateId(2), 1)]);
    }

    /// 契約5: 物流比率が同じとき、州登録順に依存せず決定的に選ぶ(StateId最小)。
    #[test]
    fn p28_005_tie_breaks_by_state_id_regardless_of_registration_order() {
        let build = |states: Vec<StateData>| {
            let country = p28_country(CountryId(1), StateId(1), 5000.0);
            p28_005_queued_railways(&p28_run(country, states, None).0)
        };
        let forward = build(vec![
            p28_005_state(2, CountryId(1), 0.5),
            p28_005_state(5, CountryId(1), 0.5),
            p28_005_state(9, CountryId(1), 0.5),
        ]);
        let reversed = build(vec![
            p28_005_state(9, CountryId(1), 0.5),
            p28_005_state(5, CountryId(1), 0.5),
            p28_005_state(2, CountryId(1), 0.5),
        ]);
        assert_eq!(forward, vec![(StateId(2), 1)]);
        assert_eq!(forward, reversed, "登録順を反転しても同じ州(StateId最小)を選ぶ");
    }

    /// 契約6: 自国所有・支配の条件を満たさない州(他国が所有、または自国所有だが
    /// 敵が占領中)には着工しない。
    #[test]
    fn p28_005_never_targets_a_state_the_country_does_not_own_or_control() {
        let foreign = p28_005_state(1, CountryId(2), 0.1); // 他国所有
        let mut occupied = p28_005_state(2, CountryId(1), 0.1); // 自国所有・敵が占領中
        occupied.controller_country = Some(CountryId(2));
        let own = p28_005_state(3, CountryId(1), 0.9); // 唯一の適格州(物流0.9で非対象)
        let country = p28_country(CountryId(1), StateId(3), 5000.0);
        let (after, _) = p28_run(country, vec![foreign, occupied, own], None);
        assert!(
            p28_005_queued_railways(&after).is_empty(),
            "所有かつ支配する州以外はどれだけ物流が低くても対象にならない"
        );
    }

    /// 契約7: 既存の建設キューと建設可能条件を尊重する
    /// (重複発注しない、最大レベルは対象外、キュー上限2件も既存どおり)。
    #[test]
    fn p28_005_respects_existing_queue_and_buildable_conditions() {
        // 既にRailwayがキュー中の州は、物流が低くても再発注しない。
        let mut country = p28_country(CountryId(1), StateId(1), 5000.0);
        country.construction_queue.push(ConstructionQueueItem {
            state_id: StateId(1),
            building_type: BuildingType::Railway,
            target_level: 1,
            progress: 0.0,
            required_progress: 60.0,
            paid_cost: 800.0,
            status: ConstructionStatus::InQueue,
        });
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert_eq!(
            p28_005_queued_railways(&after),
            vec![(StateId(1), 1)],
            "既存の1件だけが残り、重複発注しない(この州が唯一の候補なので優先度2.7はfalseを返し、\
             優先度3のFactoryへフォールスルーして別途1件追加されるのは契約11どおりの正しい動作)"
        );

        // 最大レベル(テスト用定義では5)に達した州は対象外。
        let mut maxed = p28_005_state(2, CountryId(1), 0.1);
        maxed.buildings.insert(BuildingType::Railway, 5);
        let country = p28_country(CountryId(1), StateId(2), 5000.0);
        let (after, _) = p28_run(country, vec![maxed], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "最大レベルの州には着工しない");

        // キュー上限2件は既存の冒頭判定のまま(Railway専用の例外を作らない)。
        let mut country = p28_country(CountryId(1), StateId(1), 5000.0);
        country.construction_queue.push(ConstructionQueueItem {
            state_id: StateId(9),
            building_type: BuildingType::Factory,
            target_level: 1,
            progress: 0.0,
            required_progress: 10.0,
            paid_cost: 300.0,
            status: ConstructionStatus::InQueue,
        });
        country.construction_queue.push(ConstructionQueueItem {
            state_id: StateId(9),
            building_type: BuildingType::Mine,
            target_level: 1,
            progress: 0.0,
            required_progress: 10.0,
            paid_cost: 500.0,
            status: ConstructionStatus::InQueue,
        });
        let before_treasury = country.treasury;
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert_eq!(after.construction_queue.len(), 2, "既存のキュー上限2件を尊重する");
        assert!((after.treasury - before_treasury).abs() < 1e-9);
    }

    /// 契約8: 建設費 + 月次経常支出3か月分の予備資金を確保できなければ着工しない
    /// (優先度3・戦時MilitaryFactoryと同じ数値安全規約。非有限値も安全側に倒す)。
    #[test]
    fn p28_005_treasury_threshold_is_cost_plus_three_months_of_expenses() {
        // 800 + 100×3 = 1100(p28_countryのmonthly_expensesは100)。
        let country = p28_country(CountryId(1), StateId(1), 1099.999);
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "1099.999は不足");

        let country = p28_country(CountryId(1), StateId(1), 1100.0);
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert_eq!(p28_005_queued_railways(&after), vec![(StateId(1), 1)], "1100ちょうどは着工する(>=)");

        let mut country = p28_country(CountryId(1), StateId(1), f64::NAN);
        country.monthly_expenses = 100.0;
        let (after, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "国庫がNaNなら着工しない");
    }

    /// 契約9: 1回の判断でRailwayを複数着工しない(適格州が複数あっても1件だけ)。
    #[test]
    fn p28_005_builds_at_most_one_railway_per_decision() {
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let states = vec![p28_005_state(1, CountryId(1), 0.2), p28_005_state(2, CountryId(1), 0.3)];
        let (after, _) = p28_run(country, states, None);
        assert_eq!(after.construction_queue.len(), 1);
        assert_eq!(p28_005_queued_railways(&after), vec![(StateId(1), 1)]);
    }

    /// 契約10: Food緊急・供給網修復(優先度2)・戦時MilitaryFactory(優先度2.5)の
    /// 既存優先順位を維持する(Railwayに奪われない)。
    #[test]
    fn p28_005_existing_higher_priorities_are_preserved() {
        // 10-a: Food緊急(優先度1)が最優先。
        let mut hungry = p28_country(CountryId(1), StateId(1), 5000.0);
        hungry.stockpile.set(ResourceType::Food, 10.0);
        let (after, reason) = p28_run(hungry, vec![p28_005_state(1, CountryId(1), 0.1)], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "Food緊急時はRailwayより優先");
        assert_eq!(reason, CountryAiDecisionReason::FoodShortageFarmPriority);

        // 10-b: 供給網修復(優先度2、IndustrialGoodsの構造的経路がない)がRailwayより優先。
        let mut no_factory = p28_state(1, CountryId(1), 0.1);
        no_factory.buildings.insert(BuildingType::MilitaryFactory, 1);
        no_factory.building_operation_ratios.insert(BuildingType::MilitaryFactory, 1.0);
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, reason) = p28_run(country, vec![no_factory], None);
        assert!(p28_005_queued_railways(&after).is_empty(), "供給網修復がRailwayより優先");
        assert_eq!(reason, CountryAiDecisionReason::SteelShortageSteelMillPriority);

        // 10-c: 戦時MilitaryFactory(優先度2.5、実月産不足)がRailwayより優先。
        // (Factory lv1・MilitaryFactory lv1@稼働率0.2の`p28_base_state`に、
        // Railwayも対象になるよう物流比率0.1を明示的に上書きする。)
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let mut low_logistics_wartime = p28_base_state(CountryId(1));
        low_logistics_wartime.logistics_ratio = 0.1;
        let (after, reason) = p28_run(country, vec![low_logistics_wartime], Some(P28_TWO_DIVISIONS));
        assert_eq!(p28_queued_military_factories(&after), vec![(StateId(1), 2)]);
        assert!(p28_005_queued_railways(&after).is_empty(), "戦時MilitaryFactoryがRailwayより優先");
        assert_eq!(reason, CountryAiDecisionReason::WarInProgress);
    }

    /// 契約11: Railwayの判断後、条件を満たさないときは既存の優先度3
    /// (原材料不足→Mine、それ以外→Factory)へ正しく進む。
    #[test]
    fn p28_005_falls_through_to_normal_development_when_ineligible() {
        // 物流は十分(0.9以上)だが原材料が不足 → 優先度3のMine。
        let mut low_materials = p28_005_state(1, CountryId(1), 1.0);
        let mut country = p28_country(CountryId(1), StateId(1), 5000.0);
        country.stockpile.set(ResourceType::Iron, 10.0);
        low_materials.resource_deposits = vec![p26_008_c_deposit(ResourceType::Iron, true)];
        let (after, reason) = p28_run(country, vec![low_materials], None);
        assert!(p28_005_queued_railways(&after).is_empty());
        assert_eq!(reason, CountryAiDecisionReason::RawMaterialMinePriority);

        // 物流も原材料も十分 → 優先度3のFactory(既存の通常開発)。
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after, reason) = p28_run(country, vec![p28_005_state(1, CountryId(1), 1.0)], None);
        assert!(p28_005_queued_railways(&after).is_empty());
        assert_eq!(reason, CountryAiDecisionReason::PeacetimeDevelopment);
        assert!(
            after.construction_queue.iter().any(|q| q.building_type == BuildingType::Factory),
            "Railway非対象なら既存の優先度3がFactoryを建てる"
        );
    }

    /// 契約12: プレイヤー国家・消滅国家にはRailway優先建設を適用しない
    /// (`process_daily_country_ai`経由、既存の除外ロジックのまま)。
    #[test]
    fn p28_005_daily_ai_never_applies_to_player_or_eliminated_countries() {
        let (
            player_country,
            _,
            _,
            _,
            _,
            _,
            mut diplomacy_registry,
            tech_registry,
            world_state,
            mut justification_registry,
            mut frontline_registry,
            mut military_ai_registry,
            mut country_ai_registry,
        ) = setup_test_env();
        // 1=プレイヤー、2=通常のAI(平時)、3=消滅国家。いずれも物流0.1の州を持つ。
        let mut countries: Vec<CountryData> =
            (1..=3).map(|i| p28_country(CountryId(i), StateId(i), 5000.0)).collect();
        countries[2].status = crate::country::CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".into(),
            annexed_by: CountryId(2),
        };
        let mut country_registry = CountryRegistry { countries };
        let states: Vec<StateData> =
            (1..=3).map(|i| p28_005_state(i, CountryId(i), 0.1)).collect();
        let state_registry = StateRegistry::build(states);
        process_daily_country_ai(
            1,
            "1800/01/02",
            &player_country,
            &mut country_registry,
            &state_registry,
            &p26_008_c_building_registry(),
            &MilitaryRegistry::default(),
            &mut WarRegistry::default(),
            &mut diplomacy_registry,
            &tech_registry,
            &world_state,
            &mut justification_registry,
            &mut frontline_registry,
            &mut military_ai_registry,
            &mut country_ai_registry,
            &mut Vec::new(),
            &mut CrisisRegistry::default(),
        );
        let queued = |id: usize| p28_005_queued_railways(country_registry.get(CountryId(id)).unwrap());
        assert!(queued(1).is_empty(), "プレイヤー国家には適用しない");
        assert_eq!(queued(2), vec![(StateId(2), 1)], "平時の通常AI国家には適用する");
        assert!(queued(3).is_empty(), "消滅国家には適用しない");
    }

    /// 契約13: Railway完成後の次回判断は、現在の物流比率と建物レベルを再評価する
    /// (過去の判断を記憶せず、毎回作り直した`owned_states`から選び直す)。
    #[test]
    fn p28_005_reevaluates_current_logistics_and_level_after_completion() {
        // 1回目: 物流0.5の州にlv1を着工。
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after_first, _) = p28_run(country, vec![p28_005_state(1, CountryId(1), 0.5)], None);
        assert_eq!(p28_005_queued_railways(&after_first), vec![(StateId(1), 1)]);

        // 完成をシミュレート: レベル1・キューを空にし、物流はまだ0.7(<0.9)のまま。
        let mut completed_state = p28_005_state(1, CountryId(1), 0.7);
        completed_state.buildings.insert(BuildingType::Railway, 1);
        let mut after_completion = after_first;
        after_completion.construction_queue.clear();
        let (after_second, _) = p28_run(after_completion, vec![completed_state], None);
        assert_eq!(
            p28_005_queued_railways(&after_second),
            vec![(StateId(1), 2)],
            "完成後もまだ0.9未満なら、現在のレベル(1)を基にlv2を再発注する"
        );

        // 物流が0.9以上まで改善していれば、レベルに関わらずもう発注しない。
        let mut satisfied_state = p28_005_state(1, CountryId(1), 0.95);
        satisfied_state.buildings.insert(BuildingType::Railway, 1);
        let country = p28_country(CountryId(1), StateId(1), 5000.0);
        let (after_third, _) = p28_run(country, vec![satisfied_state], None);
        assert!(p28_005_queued_railways(&after_third).is_empty(), "0.9以上に改善していれば再評価で対象から外れる");
    }
}
