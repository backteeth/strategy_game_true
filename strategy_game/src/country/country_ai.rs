use crate::app::time::DayChangedMessage;
use crate::building::construction::{ConstructionQueueItem, ConstructionStatus};
use crate::building::data::BuildingType;
use crate::common::{CountryId, StateId, WarId};
use crate::country::{CountryRegistry, PlayerCountry};
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
            CountryAiDecisionReason::NoAvailableDivisions => "country_ai_reason.no_available_divisions",
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
fn compute_total_power_by_country(
    military_registry: &MilitaryRegistry,
    state_registry: &StateRegistry,
) -> Vec<u64> {
    let mut power_by_country: Vec<u64> = Vec::new();
    for division in military_registry.divisions.values() {
        if division.manpower == 0 || division.status == crate::military::data::DivisionStatus::Destroyed {
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

/// 1. 建設AI (月次評価: game_day % 30 == country_id % 30 または dirty)
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

    // 優先度判定: 食料(農場) -> 原材料(鉱山) -> 工場
    let target_building = if country.stockpile.get(ResourceType::Food) < 50.0 {
        ai_state.decision_reason = CountryAiDecisionReason::FoodShortageFarmPriority;
        BuildingType::Farm
    } else if country.stockpile.get(ResourceType::Wood) < 30.0
        || country.stockpile.get(ResourceType::Iron) < 30.0
        || country.stockpile.get(ResourceType::Coal) < 30.0
    {
        ai_state.decision_reason = CountryAiDecisionReason::RawMaterialMinePriority;
        BuildingType::Mine
    } else {
        ai_state.decision_reason = CountryAiDecisionReason::PeacetimeDevelopment;
        BuildingType::Factory
    };

    let def = match building_registry.get(target_building) {
        Some(d) => d,
        None => return,
    };

    if country.treasury < def.construction_cost {
        ai_state.decision_reason = CountryAiDecisionReason::FundsShortage;
        return;
    }

    // 候補地域を選択 (同種建物が少なく StateId 最小)
    let selected_state = owned_states.into_iter().find(|&sid| {
        let in_queue = country
            .construction_queue
            .iter()
            .any(|item| item.state_id == sid && item.building_type == target_building);
        if in_queue {
            return false;
        }
        if let Some(state_data) = state_registry.get(sid) {
            let current_level = state_data.building_level(target_building);
            current_level < def.max_level
        } else {
            false
        }
    });

    if let Some(state_id) = selected_state {
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
) {
    let player_cid = player_country.0;

    let mut ai_country_ids: Vec<CountryId> = country_registry
        .countries
        .iter()
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

    // ── P20-008 回帰テスト ───────────────────────────────────────────────────
    // `process_war_preparation_ai` のO(countries)重複計算を事前計算マップに
    // 置き換えた最適化(P20-008)が、既存の`calculate_country_total_power`単体呼び出し
    // および州支配フィルタと完全に同一の値を返すことを検証する。

    fn make_test_division(
        owner: CountryId,
        current_state: StateId,
        manpower: u64,
    ) -> crate::military::data::Division {
        use crate::military::data::{DivisionStatus, DivisionSize, DivisionType};
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
}
