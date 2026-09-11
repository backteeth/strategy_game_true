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

/// P26-004-PRE-002: 保有する生産建物の入力資源のうち、稼働中Mineがまだ存在しない
/// 資源について、対応する未活用/増設可能な領内鉱床を持つ州を1件だけ決定的に選ぶ。
/// 複数の資源が該当する場合は`ResourceType::ALL`の宣言順(Food,Wood,Iron,Coal,...)を
/// tie-breakとして用い、同一資源内ではStateId昇順を用いる(spec §5.5)。
/// MagicCrystalは通常Mineの対象外(CrystalRefinery系統の管轄)のため候補にしない。
fn find_proactive_mine_state(
    country: &CountryData,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    owned_states: &[StateId],
    mine_max_level: u32,
) -> Option<StateId> {
    let needed = resource_types_required_by_owned_buildings(state_registry, building_registry, owned_states);
    if needed.is_empty() {
        return None;
    }
    for &resource in ResourceType::ALL.iter() {
        if resource == ResourceType::MagicCrystal || !needed.contains(&resource) {
            continue;
        }
        if has_active_mine_for_resource(state_registry, owned_states, resource) {
            continue;
        }
        if let Some(sid) =
            find_undeveloped_deposit_state(country, state_registry, owned_states, mine_max_level, resource)
        {
            return Some(sid);
        }
    }
    None
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
pub fn process_construction_ai(
    country_id: CountryId,
    country_registry: &mut CountryRegistry,
    state_registry: &StateRegistry,
    building_registry: &crate::building::data::BuildingRegistry,
    ai_state: &mut CountryAiState,
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

    // 優先度2 (P26-004-PRE-002): 未活用鉱床の事前開発。既存の資源不足トリガー
    // (Wood/Iron/Coal<30)より先に評価する。建設費を支払えない場合はこの規則を
    // 発火させず(spec §5.3の前提条件)、優先度3へフォールスルーする。
    if let Some(mine_def) = building_registry.get(BuildingType::Mine)
        && country.treasury >= mine_def.construction_cost
    {
        let mine_max_level = mine_def.max_level;
        let mine_construction_cost = mine_def.construction_cost;
        let mine_required_progress = mine_def.required_progress;
        if let Some(state_id) = find_proactive_mine_state(
            country,
            state_registry,
            building_registry,
            &owned_states,
            mine_max_level,
        ) {
            ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
            let current_level = state_registry
                .get(state_id)
                .map(|s| s.building_level(BuildingType::Mine))
                .unwrap_or(0);
            country.treasury -= mine_construction_cost;
            country.construction_queue.push(ConstructionQueueItem {
                state_id,
                building_type: BuildingType::Mine,
                target_level: current_level + 1,
                progress: 0.0,
                required_progress: mine_required_progress,
                paid_cost: mine_construction_cost,
                status: ConstructionStatus::InQueue,
            });
            return;
        }
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

    match try_construct(country, state_registry, building_registry, &owned_states, target_building) {
        ConstructOutcome::Built => {}
        ConstructOutcome::InsufficientFunds => {
            ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
        }
        ConstructOutcome::NoCandidate => {
            if target_building == BuildingType::Mine {
                // P26-004-PRE-002 spec §5.2: 鉱床皆無国家ではMine候補が存在しない。
                // 同じ月の建設判断を無駄にしないよう、次候補(Factory)へフォールバック
                // する(架空の採掘能力は作らず、既存Factory建設ルールをそのまま再利用)。
                ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
                if let ConstructOutcome::InsufficientFunds = try_construct(
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
            process_construction_ai(
                country_id,
                country_registry,
                state_registry,
                building_registry,
                ai_state,
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
}
