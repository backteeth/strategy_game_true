/// P21-SAVE-002D: 検証済み`ValidatedSaveGameV1`をBevy Worldへ原子的に適用する。
///
/// このファイルは唯一、`ValidatedSaveGameV1`だけを入力として受け取る。未検証の
/// `SaveGameV1`や生のRON文字列を直接適用できる公開経路は存在しない
/// (`apply_validated_save`のシグネチャ自体がそれを構造的に保証する)。
/// `ValidatedSaveGameV1::into_inner`は002C(`validate.rs`)で`pub(crate)`として定義済みだが、
/// このラウンドではこのファイルからしか呼び出さない(利用箇所はこのファイルのみに限定する
/// という運用上の約束であり、`grep`で確認できる)。
///
/// 適用はPrepare→Commitの二段階:
/// - Prepare(`prepare_load`)は`&World`(共有参照)だけを受け取り、失敗し得る全ての処理
///   (必須Resourceの存在確認・ランタイム互換性確認・静的マスター互換性確認・
///   全Resource値の構築)をここで完了する。失敗時は`&World`しか渡していないため、
///   Worldは構造的に一切変更されない。
/// - Commit(`commit_load`)は`&mut World`(排他参照)を受け取り、`PreparedLoadGameV1`が
///   保持する完成済みResource値を`World::insert_resource`で置き換えるだけの、
///   失敗し得ない処理だけを行う(ファイルI/O・RON解析・version検証・参照検証・
///   fallibleな変換・sanitize・新規ID発行・ゲームロジック実行は一切行わない)。
///
/// 「原子的」の意味はOS/DBのトランザクションではなく、次のフレーム内適用単位を指す:
/// - Prepare失敗時は変更ゼロ
/// - Commitは`&mut World`という排他参照の下で実行されるため、他のBevy Systemが
///   途中状態を観測することはない(Rustの借用規則が保証する排他性であり、
///   本ラウンドで新たに実装した特別なロック機構ではない)
/// - Commit開始後に通常の失敗分岐を持たない
use crate::app::time::{GameDate, GamePaused, GameSpeed};
use crate::building::data::BuildingRegistry;
use crate::common::{CountryId, StateId};
use crate::country::country_ai::{CountryAiRegistry, CountryAiState};
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::map::camera::CameraDragState;
use crate::map::division_selection::{DragSelectState, SelectedDivision};
use crate::military::army::{ArmyRegistry, SelectedArmy};
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::research::data::TechnologyRegistry;
use crate::research::world_stage::WorldCivilizationState;
use crate::save::validate::{
    SaveValidationContext, ValidatedSaveGameV1, check_static_master_compatibility,
};
use crate::state::SelectedState;
use crate::state::data::StateRegistry;
use crate::ui::ActivePanel;
use crate::ui::diplomacy_panel::DiplomacyPanelState;
use crate::ui::military_panel::MilitaryPanelState;
use crate::ui::notification::NotificationHistory;
use crate::ui::peace_panel::PeacePanelState;
use crate::ui::politics_panel::PoliticsPanelState;
use crate::ui::research_panel::ResearchPanelState;
use crate::war::data::WarRegistry;
use crate::war::frontline::FrontlineRegistry;
use crate::war::justification::WarJustificationRegistry;
use crate::war::military_ai::{MilitaryAiRegistry, MilitaryAiState};
use bevy::prelude::*;
use std::collections::HashSet;

/// 適用前に発見された、Commit不可能な問題。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyLoadError {
    /// 適用先Worldに必須のランタイムResourceが存在しない。
    MissingRuntimeResource { name: &'static str },
    /// 検証時に使用した静的マスターと、適用先Worldの現在の静的マスターが一致しない
    /// (例: セーブが参照するBuildingType/技術ID/DivisionDefinitionId/WorldStageが
    /// 適用先の現在のマスターデータに存在しない)。
    StaticDataMismatch { detail: String },
    /// 適用先Worldの構造(State/Countryの集合等)が、セーブと安全に同期できない。
    RuntimeCompatibility {
        detail: String,
        /// P21-SAVE-002F1: どちらの集合(Country/State)が、具体的にどのIDで
        /// 不一致だったかを診断できる構造化情報。
        issue: RuntimeCompatibilityIssue,
    },
}

/// P21-SAVE-002F1: `RuntimeCompatibility`失敗の詳細分類。
///
/// これはマイグレーション機能ではない。異なるマップ構造のセーブを変換する経路は
/// 存在せず、あくまで「適用前に安全に拒否する」ための診断情報だけを保持する。
/// Country/Stateどちらの不一致かを識別できるようにし、それぞれについて
/// 「現在Worldには存在するが、セーブ側に欠けているID」(`missing_from_save`)と
/// 「セーブ側にだけ存在するID」(`extra_in_save`)を分けて保持する。件数だけの比較では
/// 発見できない「同数だが別ID」のケースも、この2つのVecがどちらも空でなくなることで
/// 表現される。両Vecとも、`HashSet`の反復順序に依存しないよう、格納前に必ずID値の
/// 昇順でソート済み(`diff_id_sets`を参照)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCompatibilityIssue {
    CountrySetMismatch {
        missing_from_save: Vec<CountryId>,
        extra_in_save: Vec<CountryId>,
    },
    StateSetMismatch {
        missing_from_save: Vec<StateId>,
        extra_in_save: Vec<StateId>,
    },
}

/// `current`(現在World)と`save`(セーブ)、2つのID集合を比較し、
/// (現在Worldにはあるがセーブに無いID, セーブにはあるが現在Worldに無いID)を
/// それぞれ`sort_key`の昇順でソートして返す。`HashSet::difference`自体の反復順序は
/// プロセスごとに変わり得るため、呼び出し側がこの関数の戻り値をそのままエラーへ
/// 格納することで、表示順序が決定的になることを保証する。
fn diff_id_sets<T: Copy + Eq + std::hash::Hash>(
    current: &HashSet<T>,
    save: &HashSet<T>,
    sort_key: impl Fn(&T) -> usize,
) -> (Vec<T>, Vec<T>) {
    let mut missing_from_save: Vec<T> = current.difference(save).copied().collect();
    let mut extra_in_save: Vec<T> = save.difference(current).copied().collect();
    missing_from_save.sort_by_key(|id| sort_key(id));
    extra_in_save.sort_by_key(|id| sort_key(id));
    (missing_from_save, extra_in_save)
}

/// 適用結果。失敗はPrepare段階だけを基本とし、Commit開始後に通常の失敗分岐は持たない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyLoadOutcome {
    Success,
    Failure(ApplyLoadError),
}

/// Prepareが完成させた、Commit可能なResource値一式。
///
/// 全フィールドが非公開。`prepare_load`を通過した場合だけ生成でき、`commit_load`だけが
/// 消費できる。ここに保持される値は既に妥当性が確定しているため、Commit側で
/// 失敗し得る処理を行う必要が一切ない。
pub(crate) struct PreparedLoadGameV1 {
    game_date: GameDate,
    game_speed: GameSpeed,
    player_country: PlayerCountry,
    world_civilization: WorldCivilizationState,
    country_registry: CountryRegistry,
    state_registry: StateRegistry,
    diplomacy_registry: DiplomacyRegistry,
    war_justification_registry: WarJustificationRegistry,
    war_registry: WarRegistry,
    claim_registry: ClaimRegistry,
    crisis_registry: CrisisRegistry,
    country_ai_registry: CountryAiRegistry,
    military_ai_registry: MilitaryAiRegistry,
    military_registry: MilitaryRegistry,
    battle_registry: BattleRegistry,
    army_registry: ArmyRegistry,
    frontline_registry: FrontlineRegistry,
    game_paused: GamePaused,
    // ── 一時状態の初期化(セーブに含まれないため、既定値を事前に確定させておく) ──
    selected_division: SelectedDivision,
    drag_select_state: DragSelectState,
    selected_army: SelectedArmy,
    selected_state: SelectedState,
    active_panel: ActivePanel,
    diplomacy_panel_state: DiplomacyPanelState,
    military_panel_state: MilitaryPanelState,
    peace_panel_state: PeacePanelState,
    politics_panel_state: PoliticsPanelState,
    research_panel_state: ResearchPanelState,
    camera_drag_state: CameraDragState,
    notification_history: NotificationHistory,
    pending_ai_war_declarations: crate::country::country_ai::PendingAiWarDeclarations,
}

fn require_resource<'w, T: Resource>(
    world: &'w World,
    name: &'static str,
) -> Result<&'w T, ApplyLoadError> {
    world
        .get_resource::<T>()
        .ok_or(ApplyLoadError::MissingRuntimeResource { name })
}

/// `ValidatedSaveGameV1`を消費し、Commit可能な`PreparedLoadGameV1`を構築する。
///
/// `&World`(共有参照)しか受け取らないため、失敗しても適用先Worldは一切変更されない
/// (関数シグネチャ自体が構造的な保証)。次を完了する:
/// - 必須ランタイムResourceの存在確認
/// - ランタイム互換性の確認(StateId/CountryId集合が適用先の現在の地図と一致するか)
/// - 静的マスター互換性の確認(002Cが検証時に使ったマスターと現在のマスターの一致)
/// - 各Registryの構築(`StateRegistry::build`で`index_map`を再構築、静的マスターデータの
///   移植、AI dirtyのtrue初期化を含む)
pub(crate) fn prepare_load(
    validated: ValidatedSaveGameV1,
    world: &World,
) -> Result<PreparedLoadGameV1, ApplyLoadError> {
    // 1. 必須ランタイムResourceの存在確認(欠落があれば即座に失敗、Worldは無変更)
    require_resource::<GameDate>(world, "GameDate")?;
    require_resource::<GameSpeed>(world, "GameSpeed")?;
    require_resource::<PlayerCountry>(world, "PlayerCountry")?;
    let world_civilization_current =
        require_resource::<WorldCivilizationState>(world, "WorldCivilizationState")?;
    let country_registry_current = require_resource::<CountryRegistry>(world, "CountryRegistry")?;
    let state_registry_current = require_resource::<StateRegistry>(world, "StateRegistry")?;
    require_resource::<DiplomacyRegistry>(world, "DiplomacyRegistry")?;
    require_resource::<WarJustificationRegistry>(world, "WarJustificationRegistry")?;
    require_resource::<WarRegistry>(world, "WarRegistry")?;
    require_resource::<ClaimRegistry>(world, "ClaimRegistry")?;
    require_resource::<CrisisRegistry>(world, "CrisisRegistry")?;
    require_resource::<CountryAiRegistry>(world, "CountryAiRegistry")?;
    require_resource::<MilitaryAiRegistry>(world, "MilitaryAiRegistry")?;
    let military_registry_current =
        require_resource::<MilitaryRegistry>(world, "MilitaryRegistry")?;
    require_resource::<BattleRegistry>(world, "BattleRegistry")?;
    require_resource::<ArmyRegistry>(world, "ArmyRegistry")?;
    require_resource::<FrontlineRegistry>(world, "FrontlineRegistry")?;
    require_resource::<GamePaused>(world, "GamePaused")?;
    let building_registry = require_resource::<BuildingRegistry>(world, "BuildingRegistry")?;
    let technology_registry = require_resource::<TechnologyRegistry>(world, "TechnologyRegistry")?;
    require_resource::<SelectedDivision>(world, "SelectedDivision")?;
    require_resource::<DragSelectState>(world, "DragSelectState")?;
    require_resource::<SelectedArmy>(world, "SelectedArmy")?;
    require_resource::<SelectedState>(world, "SelectedState")?;
    require_resource::<ActivePanel>(world, "ActivePanel")?;
    require_resource::<DiplomacyPanelState>(world, "DiplomacyPanelState")?;
    require_resource::<MilitaryPanelState>(world, "MilitaryPanelState")?;
    require_resource::<PeacePanelState>(world, "PeacePanelState")?;
    require_resource::<PoliticsPanelState>(world, "PoliticsPanelState")?;
    require_resource::<ResearchPanelState>(world, "ResearchPanelState")?;
    require_resource::<CameraDragState>(world, "CameraDragState")?;
    require_resource::<NotificationHistory>(world, "NotificationHistory")?;
    require_resource::<crate::country::country_ai::PendingAiWarDeclarations>(
        world,
        "PendingAiWarDeclarations",
    )?;

    let save = validated.save();

    // 2. ランタイム互換性の確認: セーブのState/CountryId集合が、適用先の現在の地図と
    //    完全に一致すること(部分一致・上位互換は許容しない。既存描画Entityは
    //    現在の地図構造を前提に1度だけ生成されるため)。`HashSet`同士の`!=`は要素の
    //    完全一致を比較するため、件数が同じでもIDが異なる場合を正しく検出する
    //    (単なる`.len()`比較ではない)。これはマイグレーション機能ではなく、
    //    Worldを変更する前に安全に拒否するためのランタイム互換性検証である
    //    (P21-SAVE-002F1)。
    let current_state_ids: HashSet<StateId> =
        state_registry_current.states.iter().map(|s| s.id).collect();
    let save_state_ids: HashSet<StateId> = save.states.iter().map(|s| s.id).collect();
    if current_state_ids != save_state_ids {
        let (missing_from_save, extra_in_save) =
            diff_id_sets(&current_state_ids, &save_state_ids, |id| id.0);
        return Err(ApplyLoadError::RuntimeCompatibility {
            detail: format!(
                "save references {} states but the currently loaded map has {}; state ID sets do not match ({} missing from save, {} extra in save)",
                save_state_ids.len(),
                current_state_ids.len(),
                missing_from_save.len(),
                extra_in_save.len(),
            ),
            issue: RuntimeCompatibilityIssue::StateSetMismatch {
                missing_from_save,
                extra_in_save,
            },
        });
    }

    let current_country_ids: HashSet<CountryId> = country_registry_current
        .countries
        .iter()
        .map(|c| c.id)
        .collect();
    let save_country_ids: HashSet<CountryId> = save.countries.iter().map(|c| c.id).collect();
    if current_country_ids != save_country_ids {
        let (missing_from_save, extra_in_save) =
            diff_id_sets(&current_country_ids, &save_country_ids, |id| id.0);
        return Err(ApplyLoadError::RuntimeCompatibility {
            detail: format!(
                "save references {} countries but the currently loaded map has {}; country ID sets do not match ({} missing from save, {} extra in save)",
                save_country_ids.len(),
                current_country_ids.len(),
                missing_from_save.len(),
                extra_in_save.len(),
            ),
            issue: RuntimeCompatibilityIssue::CountrySetMismatch {
                missing_from_save,
                extra_in_save,
            },
        });
    }

    // 3. 静的マスター互換性の確認(002Cのcontext構築経路を再利用)
    let context = SaveValidationContext {
        building_definitions: &building_registry.definitions,
        technology_definitions: &technology_registry.definitions,
        division_definitions: &military_registry_current.definitions,
        world_stage_definitions: &world_civilization_current.stage_definitions,
    };
    check_static_master_compatibility(save, &context).map_err(|errors| {
        ApplyLoadError::StaticDataMismatch {
            detail: format!(
                "{} static-master reference issue(s) against the current world, e.g. {:?}",
                errors.issues.len(),
                errors.issues.first()
            ),
        }
    })?;

    // 4. 静的マスターデータの移植に必要な値を、Worldからの借用が生きている間に複製する。
    let preserved_division_definitions = military_registry_current.definitions.clone();
    let preserved_stage_definitions = world_civilization_current.stage_definitions.clone();

    // 5. ここから先はセーブの所有権を取得し、失敗しない変換だけを行う。
    let mut save = validated.into_inner();

    // P21-009-FIX-001: 旧セーブに残るクリスタル専用州のMineをCrystalMineへ正規化する。
    // Worldへは一切触れず、所有権を取得した`save`(ローカル値)だけを変換するため、
    // 失敗し得ない純粋なデータ変換としてprepare段階(Commit前)に置ける。
    crate::building::mine_migration::migrate_mines_to_crystal_mines(
        &mut save.states,
        &mut save.countries,
        &building_registry.definitions,
    );

    let country_ai_registry = CountryAiRegistry {
        ai_states: save
            .country_ai
            .ai_states
            .into_iter()
            .map(|(id, s)| {
                (
                    id,
                    CountryAiState {
                        country_id: s.country_id,
                        mode: s.mode,
                        decision_reason: s.decision_reason,
                        last_daily_evaluation_day: s.last_daily_evaluation_day,
                        last_weekly_evaluation_day: s.last_weekly_evaluation_day,
                        last_monthly_evaluation_day: s.last_monthly_evaluation_day,
                        cooldown_until_day: s.cooldown_until_day,
                        // P21-SAVE-002A1の方針通り、ロード直後は常にdirty=trueから再開する。
                        dirty: true,
                    },
                )
            })
            .collect(),
        dirty: true,
    };

    let military_ai_registry = MilitaryAiRegistry {
        ai_states: save
            .military_ai
            .ai_states
            .into_iter()
            .map(|(id, s)| {
                (
                    id,
                    MilitaryAiState {
                        country_id: s.country_id,
                        last_evaluated_day: s.last_evaluated_day,
                        last_decision_reason: s.last_decision_reason,
                        estimated_own_power: s.estimated_own_power,
                        estimated_enemy_power: s.estimated_enemy_power,
                        dirty: true,
                    },
                )
            })
            .collect(),
        dirty: true,
    };

    Ok(PreparedLoadGameV1 {
        game_date: GameDate::from_saved_parts(
            save.date.year,
            save.date.month,
            save.date.day,
            save.date.accumulator,
        ),
        game_speed: GameSpeed(save.game_speed),
        player_country: PlayerCountry(save.player_country),
        world_civilization: WorldCivilizationState {
            current_stage: save.world_civilization.current_stage,
            stage_definitions: preserved_stage_definitions,
            milestone_countries: save.world_civilization.milestone_countries,
            last_advanced_date: save.world_civilization.last_advanced_date,
        },
        country_registry: CountryRegistry {
            countries: save.countries,
        },
        state_registry: StateRegistry::build(save.states),
        diplomacy_registry: DiplomacyRegistry {
            relations: save.diplomacy.relations,
        },
        war_justification_registry: WarJustificationRegistry::from_saved_parts(
            save.war_justifications.justifications,
            save.war_justifications.next_id,
        ),
        war_registry: WarRegistry::from_saved_parts(save.wars.wars, save.wars.next_id),
        claim_registry: ClaimRegistry::from_saved_parts(save.claims.claims, save.claims.next_id),
        crisis_registry: CrisisRegistry::from_saved_parts(save.crises.crises, save.crises.next_id),
        country_ai_registry,
        military_ai_registry,
        military_registry: MilitaryRegistry::from_saved_parts(
            preserved_division_definitions,
            save.military.divisions,
            save.military.next_division_id,
        ),
        battle_registry: BattleRegistry::from_saved_parts(
            save.battles.battles,
            save.battles.next_id,
        ),
        army_registry: ArmyRegistry::from_saved_parts(
            save.armies.armies,
            save.armies.division_army_map,
            save.armies.next_id,
            save.armies.next_army_number,
        ),
        frontline_registry: FrontlineRegistry {
            frontlines: save.frontlines.frontlines,
            plans: save.frontlines.plans,
            next_frontline_id: save.frontlines.next_frontline_id,
            division_frontline_map: save.frontlines.division_frontline_map,
            frontline_generated_movements: save.frontlines.frontline_generated_movements,
            army_frontline_map: save.frontlines.army_frontline_map,
        },
        game_paused: GamePaused(true),
        selected_division: SelectedDivision::default(),
        drag_select_state: DragSelectState::default(),
        selected_army: SelectedArmy::default(),
        selected_state: SelectedState::default(),
        active_panel: ActivePanel::default(),
        diplomacy_panel_state: DiplomacyPanelState::default(),
        military_panel_state: MilitaryPanelState::default(),
        peace_panel_state: PeacePanelState::default(),
        politics_panel_state: PoliticsPanelState::default(),
        research_panel_state: ResearchPanelState::default(),
        camera_drag_state: CameraDragState::default(),
        notification_history: NotificationHistory::default(),
        pending_ai_war_declarations: crate::country::country_ai::PendingAiWarDeclarations::default(
        ),
    })
}

/// `PreparedLoadGameV1`をWorldへ反映する。`&mut World`という排他参照の下でのみ呼ばれるため、
/// 他のBevy Systemがこの処理の途中状態を観測することはない。ここでの処理は
/// `World::insert_resource`による単純な置換だけであり、失敗し得ない
/// (ファイルI/O・RON解析・version検証・参照検証・fallibleな変換・sanitize・新規ID発行・
/// ゲームロジック実行のいずれも行わない)。
pub(crate) fn commit_load(world: &mut World, prepared: PreparedLoadGameV1) {
    world.insert_resource(prepared.game_date);
    world.insert_resource(prepared.game_speed);
    world.insert_resource(prepared.player_country);
    world.insert_resource(prepared.world_civilization);
    world.insert_resource(prepared.country_registry);
    world.insert_resource(prepared.state_registry);
    world.insert_resource(prepared.diplomacy_registry);
    world.insert_resource(prepared.war_justification_registry);
    world.insert_resource(prepared.war_registry);
    world.insert_resource(prepared.claim_registry);
    world.insert_resource(prepared.crisis_registry);
    world.insert_resource(prepared.country_ai_registry);
    world.insert_resource(prepared.military_ai_registry);
    world.insert_resource(prepared.military_registry);
    world.insert_resource(prepared.battle_registry);
    world.insert_resource(prepared.army_registry);
    world.insert_resource(prepared.frontline_registry);
    world.insert_resource(prepared.game_paused);
    world.insert_resource(prepared.selected_division);
    world.insert_resource(prepared.drag_select_state);
    world.insert_resource(prepared.selected_army);
    world.insert_resource(prepared.selected_state);
    world.insert_resource(prepared.active_panel);
    world.insert_resource(prepared.diplomacy_panel_state);
    world.insert_resource(prepared.military_panel_state);
    world.insert_resource(prepared.peace_panel_state);
    world.insert_resource(prepared.politics_panel_state);
    world.insert_resource(prepared.research_panel_state);
    world.insert_resource(prepared.camera_drag_state);
    world.insert_resource(prepared.notification_history);
    world.insert_resource(prepared.pending_ai_war_declarations);
}

/// 検証済みセーブをBevy Worldへ適用する唯一の公開API。
///
/// 受け取れる型は`ValidatedSaveGameV1`だけに限定される(未検証の`SaveGameV1`・RON文字列を
/// 直接適用する経路は存在しない)。内部でPrepare→Commitの二段階を実行し、Prepareが
/// 失敗した場合はWorldを一切変更せずに`ApplyLoadOutcome::Failure`を返す。
pub fn apply_validated_save(world: &mut World, validated: ValidatedSaveGameV1) -> ApplyLoadOutcome {
    match prepare_load(validated, world) {
        Ok(prepared) => {
            commit_load(world, prepared);
            ApplyLoadOutcome::Success
        }
        Err(error) => ApplyLoadOutcome::Failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::{BuildingDefinition, BuildingType};
    use crate::common::{
        ArmyId, BattleId, ClaimId, DiplomaticCrisisId, DivisionDefinitionId, DivisionId,
        FrontlineId, WarId,
    };
    use crate::country::country_ai::{CountryAiMode, CountryAiState, PendingAiWarDeclarations};
    use crate::country::{CountryData, EconomicSystem, GovernmentType};
    use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
    use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis};
    use crate::diplomacy::data::{DiplomaticPairKey, DiplomaticRelation};
    use crate::military::army::Army;
    use crate::military::battle::{Battle, BattleStatus};
    use crate::military::data::{
        Division, DivisionDefinition, DivisionSize, DivisionStatus, DivisionType,
    };
    use crate::research::data::{TechnologyDefinition, WorldStage};
    use crate::research::world_stage::WorldStageDefinition;
    use crate::save::dto::{
        SAVE_FORMAT_VERSION_V1, SaveGameV1, SavedArmyRegistry, SavedBattleRegistry,
        SavedClaimRegistry, SavedCountryAiRegistry, SavedCountryAiState, SavedCrisisRegistry,
        SavedDiplomacyRegistry, SavedFrontlineRegistry, SavedGameDate, SavedMilitaryAiRegistry,
        SavedMilitaryAiState, SavedMilitaryRegistry, SavedWarJustificationRegistry,
        SavedWarRegistry, SavedWorldCivilizationState,
    };
    use crate::save::export::{SaveGameSources, build_save_game_v1};
    use crate::save::validate::validate_save_game_v1;
    use crate::state::data::StateData;
    use crate::ui::PanelKind;
    use crate::war::data::War;
    use crate::war::frontline::{Frontline, FrontlinePlan, FrontlineStance};
    use crate::war::military_ai::MilitaryAiDecisionReason;
    use std::collections::{HashMap, HashSet};

    struct StaticData {
        buildings: HashMap<BuildingType, BuildingDefinition>,
        technologies: HashMap<String, TechnologyDefinition>,
        divisions: HashMap<DivisionDefinitionId, DivisionDefinition>,
        world_stages: HashMap<WorldStage, WorldStageDefinition>,
    }

    fn building_definition() -> BuildingDefinition {
        BuildingDefinition {
            building_type: BuildingType::Farm,
            name: "Farm".to_string(),
            construction_cost: 100.0,
            required_progress: 10.0,
            required_workforce: 10.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: HashMap::new(),
            maintenance_cost: 1.0,
            max_level: 5,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn division_definition() -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(0),
            name: "Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 1000,
            required_equipment: 10.0,
            recruitment_days: 10,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 5.0,
            organization: 100.0,
            morale: 1.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn static_data() -> StaticData {
        let mut buildings = HashMap::new();
        buildings.insert(BuildingType::Farm, building_definition());

        let mut technologies = HashMap::new();
        technologies.insert(
            "tech_a".to_string(),
            TechnologyDefinition {
                id: "tech_a".to_string(),
                name: "Tech A".to_string(),
                description: String::new(),
                field: crate::research::data::TechnologyField::Science,
                cost: 100.0,
                prerequisites: Vec::new(),
                minimum_world_stage: WorldStage::PreIndustrial,
                effects: Vec::new(),
                is_world_milestone: false,
                milestone_target_stage: None,
                ui_order: 0,
            },
        );

        let mut divisions = HashMap::new();
        divisions.insert(DivisionDefinitionId(0), division_definition());

        let mut world_stages = HashMap::new();
        world_stages.insert(
            WorldStage::PreIndustrial,
            WorldStageDefinition {
                stage: WorldStage::PreIndustrial,
                display_name: "Pre-Industrial".to_string(),
                description: String::new(),
                required_previous_stage: None,
                milestone_technologies: Vec::new(),
                required_country_count: 1,
            },
        );

        StaticData {
            buildings,
            technologies,
            divisions,
            world_stages,
        }
    }

    fn context(s: &StaticData) -> SaveValidationContext<'_> {
        SaveValidationContext {
            building_definitions: &s.buildings,
            technology_definitions: &s.technologies,
            division_definitions: &s.divisions,
            world_stage_definitions: &s.world_stages,
        }
    }

    /// 適用によって上書きされるべき「現在の(旧)World」。セーブとは意図的に異なる値を
    /// 持たせることで、適用後の値が本当にセーブ側から来ていることを検証できるようにする。
    fn build_old_world(s: &StaticData) -> World {
        let mut world = World::new();

        let mut old_date = GameDate::new(1850, 5, 10);
        old_date.add_accumulator(0.9);
        world.insert_resource(old_date);
        world.insert_resource(GameSpeed(3));
        world.insert_resource(PlayerCountry(Some(CountryId(0))));

        world.insert_resource(WorldCivilizationState {
            current_stage: WorldStage::PreIndustrial,
            stage_definitions: s.world_stages.clone(),
            milestone_countries: HashMap::new(),
            last_advanced_date: "1850/01/01".to_string(),
        });

        let country0_old = CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            treasury: 999.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };
        let country1_old = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            treasury: 111.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };
        world.insert_resource(CountryRegistry {
            countries: vec![country0_old, country1_old],
        });

        let state0 = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(1)],
            ..StateData::default()
        };
        let state1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            neighbors: vec![StateId(0)],
            ..StateData::default()
        };
        world.insert_resource(StateRegistry::build(vec![state0, state1]));

        world.insert_resource(DiplomacyRegistry::default());
        world.insert_resource(WarJustificationRegistry::default());
        world.insert_resource(WarRegistry::default());
        world.insert_resource(ClaimRegistry::default());
        world.insert_resource(CrisisRegistry::default());

        let mut old_country_ai = CountryAiRegistry::default();
        old_country_ai
            .ai_states
            .insert(CountryId(1), CountryAiState::new(CountryId(1)));
        world.insert_resource(old_country_ai);

        let mut old_military_ai = MilitaryAiRegistry::default();
        old_military_ai.ai_states.insert(
            CountryId(1),
            crate::war::military_ai::MilitaryAiState::new(CountryId(1)),
        );
        world.insert_resource(old_military_ai);

        world.insert_resource(MilitaryRegistry::from_saved_parts(
            s.divisions.clone(),
            HashMap::new(),
            50,
        ));
        world.insert_resource(BattleRegistry::default());
        world.insert_resource(ArmyRegistry::default());
        world.insert_resource(FrontlineRegistry::default());
        world.insert_resource(GamePaused(false));
        world.insert_resource(BuildingRegistry {
            definitions: s.buildings.clone(),
        });
        world.insert_resource(TechnologyRegistry {
            definitions: s.technologies.clone(),
            sorted_ids: vec!["tech_a".to_string()],
        });

        world.insert_resource(SelectedDivision {
            division_ids: [DivisionId(999)].into_iter().collect(),
        });
        world.insert_resource(DragSelectState {
            press_start_screen: None,
            is_dragging: true,
        });
        world.insert_resource(SelectedArmy(Some(ArmyId(999))));
        world.insert_resource(SelectedState(Some(StateId(0))));
        world.insert_resource(ActivePanel {
            current: PanelKind::Military,
        });
        world.insert_resource(DiplomacyPanelState {
            open: true,
            target_country: Some(CountryId(1)),
            claim_target_state: Some(StateId(0)),
            pending_crisis_claim: Some(crate::common::ClaimId(0)),
        });
        world.insert_resource(MilitaryPanelState { open: true });
        world.insert_resource(PeacePanelState {
            open: true,
            selected_war_id: Some(WarId(999)),
        });
        world.insert_resource(PoliticsPanelState { open: true });
        world.insert_resource(ResearchPanelState {
            open: true,
            active_tab: crate::research::data::TechnologyField::Military,
        });
        world.insert_resource(CameraDragState {
            drag_start: Some(Vec2::ZERO),
            camera_start: Some(Vec2::ZERO),
        });
        world.insert_resource(NotificationHistory {
            recent: vec!["old notification".to_string()],
        });
        world.insert_resource(PendingAiWarDeclarations(vec![(
            CountryId(0),
            CountryId(1),
            StateId(0),
            WarId(999),
        )]));

        world
    }

    /// 適用先の現在の地図・静的マスターと一致する、代表的な`SaveGameV1`。
    fn representative_save() -> SaveGameV1 {
        let country0 = CountryData {
            id: CountryId(0),
            capital_state_id: StateId(0),
            treasury: 5000.0,
            government_type: GovernmentType::Monarchy,
            economic_system: EconomicSystem::Mercantilism,
            ..CountryData::default()
        };
        let country1 = CountryData {
            id: CountryId(1),
            capital_state_id: StateId(1),
            treasury: 3000.0,
            government_type: GovernmentType::Republic,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        };

        let state0 = StateData {
            id: StateId(0),
            owner_country_id: CountryId(0),
            neighbors: vec![StateId(1)],
            ..StateData::default()
        };
        let state1 = StateData {
            id: StateId(1),
            owner_country_id: CountryId(1),
            controller_country: Some(CountryId(0)),
            occupation_progress: 40.0,
            neighbors: vec![StateId(0)],
            ..StateData::default()
        };
        let states = StateRegistry::build(vec![state0, state1]).states;

        let division_moving = Division {
            id: DivisionId(7),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(1),
            destination: Some(StateId(0)),
            current_path: vec![StateId(0)],
            target_state: None,
            manpower: 8000,
            max_manpower: 10_000,
            equipment: 90.0,
            max_equipment: 100.0,
            organization: 60.0,
            max_organization: 100.0,
            morale: 0.8,
            max_morale: 1.0,
            experience: 2.0,
            supply_ratio: 0.9,
            movement_progress: 0.35,
            status: DivisionStatus::Moving,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        };
        let division_fighting = Division {
            id: DivisionId(8),
            owner: CountryId(1),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 4000,
            max_manpower: 10_000,
            equipment: 50.0,
            max_equipment: 100.0,
            organization: 30.0,
            max_organization: 100.0,
            morale: 0.5,
            max_morale: 1.0,
            experience: 1.0,
            supply_ratio: 0.7,
            movement_progress: 0.0,
            status: DivisionStatus::Fighting,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: Some(BattleId(0)),
        };
        let mut divisions = HashMap::new();
        divisions.insert(division_moving.id, division_moving);
        divisions.insert(division_fighting.id, division_fighting);

        let battle = Battle {
            id: BattleId(0),
            war_id: WarId(0),
            state_id: StateId(0),
            attacker_country: CountryId(0),
            defender_country: CountryId(1),
            attacker_division_ids: Vec::new(),
            defender_division_ids: vec![DivisionId(8)],
            attacker_origins: HashMap::new(),
            start_date: "1801/06/10".to_string(),
            elapsed_days: 2,
            status: BattleStatus::Ongoing,
        };
        let mut battles = HashMap::new();
        battles.insert(battle.id, battle);

        let army = Army {
            id: ArmyId(2),
            owner: CountryId(1),
            name: "Army 3".to_string(),
            member_division_ids: vec![DivisionId(7), DivisionId(8)],
        };
        let mut armies = HashMap::new();
        armies.insert(army.id, army);
        let mut division_army_map = HashMap::new();
        division_army_map.insert(DivisionId(7), ArmyId(2));
        division_army_map.insert(DivisionId(8), ArmyId(2));
        let mut next_army_number = HashMap::new();
        next_army_number.insert(CountryId(1), 4u32);

        let war = War {
            id: WarId(0),
            name: "Test War".to_string(),
            attackers: [CountryId(0)].into_iter().collect(),
            defenders: [CountryId(1)].into_iter().collect(),
            war_goals: Vec::new(),
            start_date: "1801/06/01".to_string(),
            end_date: None,
            duration_days: 14,
            war_score: 5.0,
            attacker_war_exhaustion: 2.0,
            defender_war_exhaustion: 8.0,
            occupied_states: HashSet::new(),
            status: crate::war::data::WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        };
        let mut wars = HashMap::new();
        wars.insert(war.id, war);

        let mut relations = HashMap::new();
        relations.insert(
            DiplomaticPairKey(CountryId(0), CountryId(1)),
            DiplomaticRelation {
                opinion: -50.0,
                tension: 20.0,
                trust: 30.0,
                threat: 10.0,
                at_war: true,
                has_military_access: false,
                truce_until: None,
                treaties: Vec::new(),
                cooldowns: HashMap::new(),
                active_activity: None,
                last_updated_date: "1801/06/01".to_string(),
            },
        );

        let mut country_ai_states = HashMap::new();
        country_ai_states.insert(
            CountryId(1),
            SavedCountryAiState {
                country_id: CountryId(1),
                mode: CountryAiMode::AtWar,
                decision_reason: crate::country::country_ai::CountryAiDecisionReason::WarInProgress,
                last_daily_evaluation_day: 10,
                last_weekly_evaluation_day: 7,
                last_monthly_evaluation_day: 0,
                cooldown_until_day: 0,
            },
        );

        let mut military_ai_states = HashMap::new();
        military_ai_states.insert(
            CountryId(1),
            SavedMilitaryAiState {
                country_id: CountryId(1),
                last_evaluated_day: 10,
                last_decision_reason: MilitaryAiDecisionReason::EnemyStronger,
                estimated_own_power: 4000,
                estimated_enemy_power: 6000,
            },
        );

        let mut milestone_countries = HashMap::new();
        milestone_countries.insert(WorldStage::PreIndustrial, [0usize].into_iter().collect());

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
                current_stage: WorldStage::PreIndustrial,
                milestone_countries,
                last_advanced_date: "1801/01/01".to_string(),
            },
            countries: vec![country0, country1],
            states,
            diplomacy: SavedDiplomacyRegistry { relations },
            war_justifications: SavedWarJustificationRegistry::default(),
            wars: SavedWarRegistry { wars, next_id: 1 },
            claims: SavedClaimRegistry::default(),
            crises: SavedCrisisRegistry::default(),
            country_ai: SavedCountryAiRegistry {
                ai_states: country_ai_states,
            },
            military_ai: SavedMilitaryAiRegistry {
                ai_states: military_ai_states,
            },
            military: SavedMilitaryRegistry {
                divisions,
                next_division_id: 9,
            },
            battles: SavedBattleRegistry {
                battles,
                next_id: 1,
            },
            armies: SavedArmyRegistry {
                armies,
                division_army_map,
                next_id: 3,
                next_army_number,
            },
            frontlines: SavedFrontlineRegistry::default(),
        }
    }

    fn validated_save(s: &StaticData) -> ValidatedSaveGameV1 {
        validate_save_game_v1(representative_save(), &context(s))
            .expect("representative_save must be valid")
    }

    // ─── 型・原子性 ──────────────────────────────────────────────────────────

    #[test]
    fn only_validated_save_game_v1_can_be_applied() {
        // apply_validated_save(world: &mut World, validated: ValidatedSaveGameV1)という
        // 関数シグネチャ自体が、未検証のSaveGameV1やRON文字列を直接適用する経路が
        // 存在しないことを保証する(ValidatedSaveGameV1はvalidate_save_game_v1を通過
        // した場合だけ得られる。フィールドは非公開、Defaultも実装しない)。
        let s = static_data();
        let mut world = build_old_world(&s);
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(outcome, ApplyLoadOutcome::Success);
    }

    /// P21-009要求テスト項目24: 不正な資源在庫(NaN)を含むセーブは
    /// `validate_save_game_v1`の時点で拒否され、`ValidatedSaveGameV1`が
    /// 一切構築されないため、`apply_validated_save`/`prepare_load`/`commit_load`へは
    /// 到達し得ない(型シグネチャ上、未検証データを適用する経路が存在しない)。
    /// よってWorldは部分変更されずに残る。
    #[test]
    fn invalid_stockpile_amount_never_reaches_apply_and_world_stays_unchanged() {
        let s = static_data();
        let world = build_old_world(&s);
        let before_treasury = world.resource::<CountryRegistry>().countries[0].treasury;

        let mut save = representative_save();
        save.countries[0].stockpile.amounts.insert(
            crate::economy::resources::ResourceType::RawMagicCrystal,
            f64::NAN,
        );

        let validation_result = validate_save_game_v1(save, &context(&s));
        assert!(
            validation_result.is_err(),
            "a save with a NaN stockpile amount must fail validation before it can be applied"
        );

        assert_eq!(
            world.resource::<CountryRegistry>().countries[0].treasury,
            before_treasury,
            "World must remain untouched: invalid data never reached apply/prepare/commit"
        );
    }

    #[test]
    fn prepare_does_not_mutate_world_until_commit() {
        let s = static_data();
        let world = build_old_world(&s);
        let before_treasury = world.resource::<CountryRegistry>().countries[0].treasury;
        let before_speed = world.resource::<GameSpeed>().0;

        let prepared = prepare_load(validated_save(&s), &world)
            .expect("prepare must succeed for a valid world");
        // prepare_loadは`&World`しか受け取らないため、Worldの変更は型システムレベルで
        // 不可能(コンパイルが通ること自体が構造的な証拠)。念のため実行後も再確認する。
        assert_eq!(
            world.resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
        drop(prepared);
    }

    #[test]
    fn missing_required_resource_fails_and_leaves_other_resources_unchanged() {
        let s = static_data();
        let mut world = build_old_world(&s);
        world.remove_resource::<BattleRegistry>();
        let before_treasury = world.resource::<CountryRegistry>().countries[0].treasury;

        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert!(matches!(
            outcome,
            ApplyLoadOutcome::Failure(ApplyLoadError::MissingRuntimeResource {
                name: "BattleRegistry"
            })
        ));
        assert_eq!(
            world.resource::<CountryRegistry>().countries[0].treasury,
            before_treasury
        );
    }

    #[test]
    fn static_master_mismatch_fails_and_preserves_current_state() {
        let s = static_data();
        let mut world = build_old_world(&s);
        // 適用先の現在のTechnologyRegistryから"tech_a"を消し、検証時のcontext(sはそのまま)と
        // 適用先Worldの静的マスターを意図的に不一致にする。
        world
            .resource_mut::<TechnologyRegistry>()
            .definitions
            .remove("tech_a");

        let mut save = representative_save();
        save.countries[1]
            .research_state
            .completed_technologies
            .insert("tech_a".to_string());
        let validated = validate_save_game_v1(save, &context(&s))
            .expect("validation uses `s`'s context, which still has tech_a");

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated);
        assert!(matches!(
            outcome,
            ApplyLoadOutcome::Failure(ApplyLoadError::StaticDataMismatch { .. })
        ));
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    #[test]
    fn map_or_render_incompatibility_is_rejected_before_applying() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut states = world.resource::<StateRegistry>().states.clone();
        states.push(StateData {
            id: StateId(2),
            owner_country_id: CountryId(0),
            ..StateData::default()
        });
        world.insert_resource(StateRegistry::build(states));

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert!(matches!(
            outcome,
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { .. })
        ));
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    // ─── P21-SAVE-002F1: Country/State ID集合のランタイム互換性検査 ─────────────
    //
    // このID集合検査自体はP21-SAVE-002D時点で既に実装されていた
    // (`map_or_render_incompatibility_is_rejected_before_applying`が既存の証拠)。
    // P21-SAVE-002F1で追加したのは、失敗時の診断情報(`RuntimeCompatibilityIssue`:
    // Country/Stateどちらの不一致か、欠落/余剰の実際のID一覧を決定的な順序で保持する)
    // であり、これはマイグレーション機能ではなく、Worldを変更する前に安全に拒否する
    // ためのランタイム互換性検証である。
    //
    // `representative_save`/`build_old_world`はWar/Army/AI等の相互参照を多数持つため、
    // 既存のCountryId/StateIdを削除・付け替えると無関係な002C検証エラーを誘発してしまう。
    // 代わりに、他の何からも参照されない新しいCountryId/StateIdを追加・削除することで、
    // ID集合だけを自由に組み替える。

    /// 既存の何からも参照されない、自己完結したCountryDataを追加する
    /// (`capital_state_id`は`world`/`save`どちらにも既に存在するStateId(0)を指す)。
    /// `world`側にだけ使う(`world`は`validate_save_game_v1`を通らないため、
    /// 首都の所有者整合性は検証されない)。`save`側へ追加する場合は
    /// `push_country_with_owned_capital`を使うこと。
    fn push_unreferenced_country(countries: &mut Vec<CountryData>, id: CountryId) {
        countries.push(CountryData {
            id,
            capital_state_id: StateId(0),
            government_type: GovernmentType::Republic,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        });
    }

    /// `save`側へ、自己完結した新規Country+その首都Stateのペアを追加する。
    /// 002Cの`validate_capital`は「首都は自国が所有する州でなければならない」ことを
    /// 要求するため、`save`側ではCountryだけを単独追加すると検証に失敗する
    /// (`push_unreferenced_country`は`world`側専用)。呼び出し側は、State集合の比較が
    /// 意図せず不一致にならないよう、同じ`capital_state_id`を`world`側の
    /// StateRegistryへも(`push_unreferenced_state`等で)追加しておくこと。
    fn push_country_with_owned_capital(
        countries: &mut Vec<CountryData>,
        states: &mut Vec<StateData>,
        id: CountryId,
        capital_state_id: StateId,
    ) {
        countries.push(CountryData {
            id,
            capital_state_id,
            government_type: GovernmentType::Republic,
            economic_system: EconomicSystem::FreeMarket,
            ..CountryData::default()
        });
        states.push(StateData {
            id: capital_state_id,
            owner_country_id: id,
            ..StateData::default()
        });
    }

    /// 既存の何からも参照されない、自己完結したStateDataを追加する
    /// (`owner_country_id`は`world`/`save`どちらにも既に存在するCountryId(0)を指す)。
    fn push_unreferenced_state(states: &mut Vec<StateData>, id: StateId) {
        states.push(StateData {
            id,
            owner_country_id: CountryId(0),
            ..StateData::default()
        });
    }

    fn world_country_ids(world: &World) -> Vec<CountryId> {
        let mut ids: Vec<CountryId> = world
            .resource::<CountryRegistry>()
            .countries
            .iter()
            .map(|c| c.id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    fn world_state_ids(world: &World) -> Vec<StateId> {
        let mut ids: Vec<StateId> = world
            .resource::<StateRegistry>()
            .states
            .iter()
            .map(|s| s.id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    // 1. Country/State集合が完全一致すれば成功する。
    #[test]
    fn matching_country_and_state_id_sets_succeed() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(outcome, ApplyLoadOutcome::Success);
    }

    // 2. Vecの並び順だけが違っても(集合として同じなら)成功する。
    #[test]
    fn vector_order_alone_does_not_affect_compatibility() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut save = representative_save();
        save.countries.reverse();
        save.states.reverse();
        let validated = validate_save_game_v1(save, &context(&s))
            .expect("reversing element order must not affect internal validation");

        let outcome = apply_validated_save(&mut world, validated);
        assert_eq!(outcome, ApplyLoadOutcome::Success);
    }

    // 3. Countryが現在Worldにはあるがセーブ側で1件不足していると失敗する。
    #[test]
    fn country_missing_from_save_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut countries = world.resource::<CountryRegistry>().countries.clone();
        push_unreferenced_country(&mut countries, CountryId(2));
        world.insert_resource(CountryRegistry { countries });

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::CountrySetMismatch {
                        missing_from_save: vec![CountryId(2)],
                        extra_in_save: vec![],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(CountrySetMismatch), got {other:?}"),
        }
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    // 4. セーブ側にだけCountryが1件余分にあると失敗する。
    #[test]
    fn country_extra_in_save_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        // State集合は一致させたまま(同じStateId(50)をworld側にも追加)、
        // Countryだけをsave側にのみ追加する。
        let mut world_states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut world_states, StateId(50));
        world.insert_resource(StateRegistry::build(world_states));

        let mut save = representative_save();
        push_country_with_owned_capital(
            &mut save.countries,
            &mut save.states,
            CountryId(2),
            StateId(50),
        );
        let validated = validate_save_game_v1(save, &context(&s)).expect(
            "an unreferenced extra country with its own owned capital must not fail internal validation",
        );

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated);
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::CountrySetMismatch {
                        missing_from_save: vec![],
                        extra_in_save: vec![CountryId(2)],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(CountrySetMismatch), got {other:?}"),
        }
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    // 5. Countryの件数は同じでも、IDが別であれば失敗する(単なる件数比較では検出できない)。
    #[test]
    fn country_same_count_different_ids_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut countries = world.resource::<CountryRegistry>().countries.clone();
        push_unreferenced_country(&mut countries, CountryId(2));
        world.insert_resource(CountryRegistry { countries });
        // State集合は一致させたまま(save側の新規首都と同じStateId(50)をworld側にも追加)。
        let mut world_states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut world_states, StateId(50));
        world.insert_resource(StateRegistry::build(world_states));

        let mut save = representative_save();
        push_country_with_owned_capital(
            &mut save.countries,
            &mut save.states,
            CountryId(3),
            StateId(50),
        );
        let validated = validate_save_game_v1(save, &context(&s)).unwrap();

        assert_eq!(
            world_country_ids(&world).len(),
            3,
            "world and save must have the same country COUNT for this test to be meaningful"
        );
        let outcome = apply_validated_save(&mut world, validated);
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::CountrySetMismatch {
                        missing_from_save: vec![CountryId(2)],
                        extra_in_save: vec![CountryId(3)],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(CountrySetMismatch), got {other:?}"),
        }
    }

    // 6. Stateが現在Worldにはあるがセーブ側で1件不足していると失敗する
    //    (既存の`map_or_render_incompatibility_is_rejected_before_applying`と同じ方向だが、
    //    ここでは構造化された`issue`の中身まで確認する)。
    #[test]
    fn state_missing_from_save_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut states, StateId(2));
        world.insert_resource(StateRegistry::build(states));

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::StateSetMismatch {
                        missing_from_save: vec![StateId(2)],
                        extra_in_save: vec![],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(StateSetMismatch), got {other:?}"),
        }
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    // 7. セーブ側にだけStateが1件余分にあると失敗する。
    #[test]
    fn state_extra_in_save_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut save = representative_save();
        push_unreferenced_state(&mut save.states, StateId(2));
        let validated = validate_save_game_v1(save, &context(&s))
            .expect("an unreferenced extra state must not fail internal validation");

        let before_speed = world.resource::<GameSpeed>().0;
        let outcome = apply_validated_save(&mut world, validated);
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::StateSetMismatch {
                        missing_from_save: vec![],
                        extra_in_save: vec![StateId(2)],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(StateSetMismatch), got {other:?}"),
        }
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
    }

    // 8. Stateの件数は同じでも、IDが別であれば失敗する。
    #[test]
    fn state_same_count_different_ids_is_rejected() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut states, StateId(2));
        world.insert_resource(StateRegistry::build(states));

        let mut save = representative_save();
        push_unreferenced_state(&mut save.states, StateId(3));
        let validated = validate_save_game_v1(save, &context(&s)).unwrap();

        assert_eq!(
            world_state_ids(&world).len(),
            3,
            "world and save must have the same state COUNT for this test to be meaningful"
        );
        let outcome = apply_validated_save(&mut world, validated);
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { issue, .. }) => {
                assert_eq!(
                    issue,
                    RuntimeCompatibilityIssue::StateSetMismatch {
                        missing_from_save: vec![StateId(2)],
                        extra_in_save: vec![StateId(3)],
                    }
                );
            }
            other => panic!("expected RuntimeCompatibility(StateSetMismatch), got {other:?}"),
        }
    }

    // 9. CountryとState、両方の不一致がそれぞれ独立して診断できる
    //    (現在の実装ではState側を先に検査するため、両方同時に不一致な場合は
    //    StateSetMismatchが報告される。Stateだけを一致させればCountrySetMismatchが
    //    正しく報告されることも確認し、どちらの経路も到達可能であることを示す)。
    #[test]
    fn country_and_state_mismatches_are_each_independently_diagnosable() {
        let s = static_data();

        // 9a. 両方不一致 → 先に検査されるStateSetMismatchが報告される。
        {
            let mut world = build_old_world(&s);
            let mut countries = world.resource::<CountryRegistry>().countries.clone();
            push_unreferenced_country(&mut countries, CountryId(2));
            world.insert_resource(CountryRegistry { countries });
            let mut states = world.resource::<StateRegistry>().states.clone();
            push_unreferenced_state(&mut states, StateId(2));
            world.insert_resource(StateRegistry::build(states));

            let outcome = apply_validated_save(&mut world, validated_save(&s));
            assert!(matches!(
                outcome,
                ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility {
                    issue: RuntimeCompatibilityIssue::StateSetMismatch { .. },
                    ..
                })
            ));
        }

        // 9b. Stateは一致・Countryだけ不一致 → CountrySetMismatchが報告される。
        {
            let mut world = build_old_world(&s);
            let mut countries = world.resource::<CountryRegistry>().countries.clone();
            push_unreferenced_country(&mut countries, CountryId(2));
            world.insert_resource(CountryRegistry { countries });

            let outcome = apply_validated_save(&mut world, validated_save(&s));
            match outcome {
                ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility {
                    issue, ..
                }) => {
                    assert_eq!(
                        issue,
                        RuntimeCompatibilityIssue::CountrySetMismatch {
                            missing_from_save: vec![CountryId(2)],
                            extra_in_save: vec![],
                        }
                    );
                }
                other => panic!("expected RuntimeCompatibility(CountrySetMismatch), got {other:?}"),
            }
        }
    }

    // 10. RuntimeCompatibility失敗時、全17個の正規Resourceが1つも変更されない。
    #[test]
    fn runtime_compatibility_failure_leaves_all_normative_resources_unchanged() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut states, StateId(2));
        world.insert_resource(StateRegistry::build(states));

        let before_date = world.resource::<GameDate>().clone();
        let before_speed = world.resource::<GameSpeed>().0;
        let before_player = world.resource::<PlayerCountry>().0;
        let before_stage = world.resource::<WorldCivilizationState>().current_stage;
        let before_country_count = world.resource::<CountryRegistry>().countries.len();
        let before_state_count = world.resource::<StateRegistry>().states.len();
        let before_diplomacy = world.resource::<DiplomacyRegistry>().relations.len();
        let before_war_just = world
            .resource::<WarJustificationRegistry>()
            .justifications
            .len();
        let before_wars = world.resource::<WarRegistry>().wars.len();
        let before_claims = world.resource::<ClaimRegistry>().claims.len();
        let before_crises = world.resource::<CrisisRegistry>().crises.len();
        let before_country_ai = world.resource::<CountryAiRegistry>().ai_states.len();
        let before_military_ai = world.resource::<MilitaryAiRegistry>().ai_states.len();
        let before_divisions = world.resource::<MilitaryRegistry>().divisions.len();
        let before_battles = world.resource::<BattleRegistry>().battles.len();
        let before_armies = world.resource::<ArmyRegistry>().armies.len();
        let before_frontlines = world.resource::<FrontlineRegistry>().frontlines.len();

        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert!(matches!(
            outcome,
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { .. })
        ));

        assert_eq!(world.resource::<GameDate>().year, before_date.year);
        assert_eq!(world.resource::<GameSpeed>().0, before_speed);
        assert_eq!(world.resource::<PlayerCountry>().0, before_player);
        assert_eq!(
            world.resource::<WorldCivilizationState>().current_stage,
            before_stage
        );
        assert_eq!(
            world.resource::<CountryRegistry>().countries.len(),
            before_country_count
        );
        assert_eq!(
            world.resource::<StateRegistry>().states.len(),
            before_state_count
        );
        assert_eq!(
            world.resource::<DiplomacyRegistry>().relations.len(),
            before_diplomacy
        );
        assert_eq!(
            world
                .resource::<WarJustificationRegistry>()
                .justifications
                .len(),
            before_war_just
        );
        assert_eq!(world.resource::<WarRegistry>().wars.len(), before_wars);
        assert_eq!(
            world.resource::<ClaimRegistry>().claims.len(),
            before_claims
        );
        assert_eq!(
            world.resource::<CrisisRegistry>().crises.len(),
            before_crises
        );
        assert_eq!(
            world.resource::<CountryAiRegistry>().ai_states.len(),
            before_country_ai
        );
        assert_eq!(
            world.resource::<MilitaryAiRegistry>().ai_states.len(),
            before_military_ai
        );
        assert_eq!(
            world.resource::<MilitaryRegistry>().divisions.len(),
            before_divisions
        );
        assert_eq!(
            world.resource::<BattleRegistry>().battles.len(),
            before_battles
        );
        assert_eq!(world.resource::<ArmyRegistry>().armies.len(), before_armies);
        assert_eq!(
            world.resource::<FrontlineRegistry>().frontlines.len(),
            before_frontlines
        );
    }

    // 11. RuntimeCompatibility失敗時、13個の一時/UI Resourceも1つも変更されない。
    #[test]
    fn runtime_compatibility_failure_leaves_all_transient_resources_unchanged() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let mut states = world.resource::<StateRegistry>().states.clone();
        push_unreferenced_state(&mut states, StateId(2));
        world.insert_resource(StateRegistry::build(states));

        let before_game_paused = world.resource::<GamePaused>().0;
        // `SelectedDivision`/`DragSelectState`/`CameraDragState`は`Clone`/`PartialEq`を
        // 実装していない(このタスクの変更禁止範囲であるmap/camera.rsへderiveを追加
        // できないため)。個々のフィールド(いずれも`Copy`)を直接比較する。
        let before_selected_division_ids =
            world.resource::<SelectedDivision>().division_ids.clone();
        let before_drag_select_press_start = world.resource::<DragSelectState>().press_start_screen;
        let before_drag_select_is_dragging = world.resource::<DragSelectState>().is_dragging;
        let before_selected_army = world.resource::<SelectedArmy>().0;
        let before_selected_state = world.resource::<SelectedState>().0;
        let before_active_panel = world.resource::<ActivePanel>().current;
        let before_diplomacy_panel = world.resource::<DiplomacyPanelState>().open;
        let before_military_panel = world.resource::<MilitaryPanelState>().open;
        let before_peace_panel = world.resource::<PeacePanelState>().open;
        let before_politics_panel = world.resource::<PoliticsPanelState>().open;
        let before_research_panel = world.resource::<ResearchPanelState>().open;
        let before_camera_drag_start = world.resource::<CameraDragState>().drag_start;
        let before_camera_start = world.resource::<CameraDragState>().camera_start;
        let before_notifications = world.resource::<NotificationHistory>().recent.clone();
        let before_pending_ai_wars = world.resource::<PendingAiWarDeclarations>().0.clone();

        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert!(matches!(
            outcome,
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility { .. })
        ));

        assert_eq!(world.resource::<GamePaused>().0, before_game_paused);
        assert_eq!(
            world.resource::<SelectedDivision>().division_ids,
            before_selected_division_ids
        );
        assert_eq!(
            world.resource::<DragSelectState>().press_start_screen,
            before_drag_select_press_start
        );
        assert_eq!(
            world.resource::<DragSelectState>().is_dragging,
            before_drag_select_is_dragging
        );
        assert_eq!(world.resource::<SelectedArmy>().0, before_selected_army);
        assert_eq!(world.resource::<SelectedState>().0, before_selected_state);
        assert_eq!(world.resource::<ActivePanel>().current, before_active_panel);
        assert_eq!(
            world.resource::<DiplomacyPanelState>().open,
            before_diplomacy_panel
        );
        assert_eq!(
            world.resource::<MilitaryPanelState>().open,
            before_military_panel
        );
        assert_eq!(world.resource::<PeacePanelState>().open, before_peace_panel);
        assert_eq!(
            world.resource::<PoliticsPanelState>().open,
            before_politics_panel
        );
        assert_eq!(
            world.resource::<ResearchPanelState>().open,
            before_research_panel
        );
        assert_eq!(
            world.resource::<CameraDragState>().drag_start,
            before_camera_drag_start
        );
        assert_eq!(
            world.resource::<CameraDragState>().camera_start,
            before_camera_start
        );
        assert_eq!(
            world.resource::<NotificationHistory>().recent,
            before_notifications
        );
        assert_eq!(
            world.resource::<PendingAiWarDeclarations>().0,
            before_pending_ai_wars
        );
    }

    // 12. エラー内のID一覧は、HashSetの反復順序に関わらず常に昇順で決定的に並ぶ。
    #[test]
    fn runtime_compatibility_error_ids_are_deterministically_sorted() {
        let s = static_data();
        let mut world = build_old_world(&s);
        // 挿入順序をわざと昇順以外にする(5, 2, 9)。HashSetの反復順序に依存していれば
        // このテストは実行のたびに結果が変わり得るが、`diff_id_sets`が明示的にソートする
        // ため常に[2, 5, 9]の順で返る。
        let mut countries = world.resource::<CountryRegistry>().countries.clone();
        push_unreferenced_country(&mut countries, CountryId(5));
        push_unreferenced_country(&mut countries, CountryId(2));
        push_unreferenced_country(&mut countries, CountryId(9));
        world.insert_resource(CountryRegistry { countries });

        let outcome = apply_validated_save(&mut world, validated_save(&s));
        match outcome {
            ApplyLoadOutcome::Failure(ApplyLoadError::RuntimeCompatibility {
                issue:
                    RuntimeCompatibilityIssue::CountrySetMismatch {
                        missing_from_save, ..
                    },
                ..
            }) => {
                assert_eq!(
                    missing_from_save,
                    vec![CountryId(2), CountryId(5), CountryId(9)],
                    "missing_from_save must be sorted ascending by ID, not insertion or hash order"
                );
            }
            other => panic!("expected RuntimeCompatibility(CountrySetMismatch), got {other:?}"),
        }
    }

    #[test]
    fn commit_replaces_all_resources_consistently_in_one_step() {
        let s = static_data();
        let mut world = build_old_world(&s);
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(outcome, ApplyLoadOutcome::Success);
        // 複数の異なるResourceを同時に確認し、「一部だけ新データ・一部だけ旧データ」という
        // 半端な適用状態が存在しないことを確認する。commit_loadは`&mut World`という
        // 排他参照の下でのみ動くため、他のBevy Systemがこの間の状態を観測することはない
        // (Rustの借用規則による構造的な保証)。
        assert_eq!(world.resource::<GameSpeed>().0, 2);
        assert_eq!(world.resource::<PlayerCountry>().0, Some(CountryId(1)));
        assert_eq!(world.resource::<GameDate>().year, 1801);
        assert_eq!(
            world
                .resource::<CountryRegistry>()
                .get(CountryId(0))
                .unwrap()
                .treasury,
            5000.0
        );
        assert!(
            world
                .resource::<MilitaryRegistry>()
                .divisions
                .contains_key(&DivisionId(7))
        );
        assert!(
            world
                .resource::<ArmyRegistry>()
                .armies
                .contains_key(&ArmyId(2))
        );
    }

    #[test]
    fn structural_no_public_path_applies_unvalidated_save_directly() {
        // apply.rsが公開するのはapply_validated_save(&mut World, ValidatedSaveGameV1)だけであり、
        // SaveGameV1やRON文字列を直接受け取る`pub fn`はこのファイルに1つも存在しない
        // (このファイルのソース自体が構造的な証拠)。この関数がValidatedSaveGameV1を
        // 実際に要求することを、正常系の呼び出しで裏付ける。
        let s = static_data();
        let mut world = build_old_world(&s);
        let outcome = apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(outcome, ApplyLoadOutcome::Success);
    }

    // ─── 正規状態 ────────────────────────────────────────────────────────────

    #[test]
    fn game_date_and_accumulator_are_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let date = world.resource::<GameDate>();
        assert_eq!(date.year, 1801);
        assert_eq!(date.month, 6);
        assert_eq!(date.day, 15);
        assert!((date.accumulator() - 0.37).abs() < f64::EPSILON);
    }

    #[test]
    fn game_speed_is_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(world.resource::<GameSpeed>().0, 2);
    }

    #[test]
    fn player_country_is_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(world.resource::<PlayerCountry>().0, Some(CountryId(1)));
    }

    #[test]
    fn game_paused_is_true_after_apply() {
        let s = static_data();
        let mut world = build_old_world(&s);
        assert!(
            !world.resource::<GamePaused>().0,
            "test setup must start paused=false"
        );
        apply_validated_save(&mut world, validated_save(&s));
        assert!(world.resource::<GamePaused>().0);
    }

    #[test]
    fn country_data_is_restored_fully() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let registry = world.resource::<CountryRegistry>();
        assert_eq!(registry.countries.len(), 2);
        let c0 = registry.get(CountryId(0)).unwrap();
        assert_eq!(c0.treasury, 5000.0);
        assert_eq!(c0.government_type, GovernmentType::Monarchy);
        let c1 = registry.get(CountryId(1)).unwrap();
        assert_eq!(c1.treasury, 3000.0);
        assert_eq!(c1.government_type, GovernmentType::Republic);
    }

    #[test]
    fn state_data_is_restored_fully() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let registry = world.resource::<StateRegistry>();
        let state1 = registry.get(StateId(1)).unwrap();
        assert_eq!(state1.controller_country, Some(CountryId(0)));
        assert_eq!(state1.occupation_progress, 40.0);
    }

    #[test]
    fn state_registry_index_map_is_rebuilt() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let registry = world.resource::<StateRegistry>();
        // index_mapはprivateだが、get()がO(1)検索として機能すること自体が
        // StateRegistry::build経由での再構築の証拠になる。
        assert!(registry.get(StateId(0)).is_some());
        assert!(registry.get(StateId(1)).is_some());
    }

    #[test]
    fn diplomacy_war_justification_and_war_are_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));

        let diplomacy = world.resource::<DiplomacyRegistry>();
        let relation = diplomacy.get(CountryId(0), CountryId(1)).unwrap();
        assert_eq!(relation.opinion, -50.0);
        assert!(relation.at_war);

        let wars = world.resource::<WarRegistry>();
        assert_eq!(wars.wars.len(), 1);
        assert_eq!(wars.next_id(), 1);
        assert_eq!(
            wars.wars.get(&WarId(0)).unwrap().status,
            crate::war::data::WarStatus::Active
        );

        assert!(
            world
                .resource::<WarJustificationRegistry>()
                .justifications
                .is_empty()
        );
    }

    #[test]
    fn claim_and_crisis_and_next_ids_are_restored() {
        let s = static_data();
        let mut save = representative_save();
        let claim = TerritorialClaim {
            id: ClaimId(0),
            claimant_country: CountryId(1),
            target_state: StateId(0),
            strength: 30.0,
            created_date: "1801/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::BorderDispute,
        };
        save.claims = SavedClaimRegistry {
            claims: [(ClaimId(0), claim)].into_iter().collect(),
            next_id: 1,
        };
        let crisis = DiplomaticCrisis {
            id: DiplomaticCrisisId(0),
            initiator: CountryId(1),
            target: CountryId(0),
            war_goals: Vec::new(),
            start_date: "1801/01/01".to_string(),
            current_phase: CrisisPhase::Negotiating,
            escalation: 10.0,
            initiator_support: 20.0,
            target_resistance: 30.0,
            days_in_phase: 1,
            deadline_date: None,
            international_concern: 5.0,
            third_party_reactions: HashMap::new(),
        };
        save.crises = SavedCrisisRegistry {
            crises: [(DiplomaticCrisisId(0), crisis)].into_iter().collect(),
            next_id: 1,
        };
        let validated = validate_save_game_v1(save, &context(&s)).unwrap();

        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated);

        let claims = world.resource::<ClaimRegistry>();
        assert_eq!(claims.claims.len(), 1);
        assert_eq!(claims.next_id(), 1);
        let crises = world.resource::<CrisisRegistry>();
        assert_eq!(crises.crises.len(), 1);
        assert_eq!(crises.next_id(), 1);
    }

    #[test]
    fn division_and_next_division_id_are_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let mil = world.resource::<MilitaryRegistry>();
        assert_eq!(mil.divisions.len(), 2);
        assert_eq!(mil.next_division_id(), 9);
        let moving = mil.divisions.get(&DivisionId(7)).unwrap();
        assert_eq!(moving.status, DivisionStatus::Moving);
    }

    #[test]
    fn battle_and_next_id_are_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let battles = world.resource::<BattleRegistry>();
        assert_eq!(battles.battles.len(), 1);
        assert_eq!(battles.next_id(), 1);
        assert_eq!(
            battles.battles.get(&BattleId(0)).unwrap().status,
            BattleStatus::Ongoing
        );
    }

    #[test]
    fn army_all_four_fields_are_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let armies = world.resource::<ArmyRegistry>();
        assert_eq!(armies.armies.len(), 1);
        assert_eq!(
            armies.division_army_map.get(&DivisionId(7)),
            Some(&ArmyId(2))
        );
        assert_eq!(armies.next_id(), 3);
        assert_eq!(armies.next_army_number_map().get(&CountryId(1)), Some(&4));
    }

    #[test]
    fn frontline_all_six_fields_are_restored() {
        let s = static_data();
        let mut save = representative_save();
        let frontline = Frontline {
            frontline_id: FrontlineId(0),
            war_id: WarId(0),
            attacker_country_id: CountryId(0),
            defender_country_id: CountryId(1),
            attacker_front_regions: vec![StateId(0)],
            defender_front_regions: vec![StateId(1)],
            border_region_pairs: vec![(StateId(0), StateId(1))],
        };
        let plan = FrontlinePlan {
            frontline_id: FrontlineId(0),
            commanding_country_id: CountryId(1),
            stance: FrontlineStance::Defend,
            objective_region_id: Some(StateId(0)),
            assigned_division_ids: vec![DivisionId(7)],
            // ArmyId(2)はこのfixtureのrepresentative_save()自体がCountryId(1)所有として
            // 用意している既存Army(save.armies.armies参照)。
            assigned_army_ids: vec![ArmyId(2)],
            offensive_line_region_ids: None,
        };
        save.frontlines = SavedFrontlineRegistry {
            frontlines: [(FrontlineId(0), frontline)].into_iter().collect(),
            plans: [((FrontlineId(0), CountryId(1)), plan)]
                .into_iter()
                .collect(),
            next_frontline_id: 1,
            division_frontline_map: [(DivisionId(7), FrontlineId(0))].into_iter().collect(),
            frontline_generated_movements: [DivisionId(7)].into_iter().collect(),
            army_frontline_map: [(ArmyId(2), FrontlineId(0))].into_iter().collect(),
        };
        let validated = validate_save_game_v1(save, &context(&s)).unwrap();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated);

        let fl = world.resource::<FrontlineRegistry>();
        assert_eq!(fl.frontlines.len(), 1);
        assert_eq!(fl.plans.len(), 1);
        assert_eq!(fl.next_frontline_id, 1);
        assert_eq!(
            fl.division_frontline_map.get(&DivisionId(7)),
            Some(&FrontlineId(0))
        );
        assert!(fl.frontline_generated_movements.contains(&DivisionId(7)));
        assert_eq!(
            fl.army_frontline_map.get(&ArmyId(2)),
            Some(&FrontlineId(0)),
            "P21-005: army_frontline_map must be restored from the saved DTO"
        );
        let plan = fl.get_plan(FrontlineId(0), CountryId(1)).unwrap();
        assert_eq!(plan.assigned_army_ids, vec![ArmyId(2)]);
    }

    #[test]
    fn world_civilization_mutable_part_is_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let wcs = world.resource::<WorldCivilizationState>();
        assert_eq!(wcs.current_stage, WorldStage::PreIndustrial);
        assert_eq!(wcs.last_advanced_date, "1801/01/01");
        assert!(
            wcs.milestone_countries
                .get(&WorldStage::PreIndustrial)
                .unwrap()
                .contains(&0)
        );
    }

    // ─── 静的・派生状態 ──────────────────────────────────────────────────────

    #[test]
    fn static_definitions_are_not_changed_by_apply() {
        // BuildingDefinition/TechnologyDefinition/DivisionDefinition/WorldStageDefinitionは
        // いずれもPartialEqを実装していない(セーブテストのためだけに既存コア型へ
        // PartialEqを追加しない方針、P21-SAVE-002A以降一貫)。キー集合の一致で
        // 「静的マスターが変更されていないこと」を確認する。
        let s = static_data();
        let mut world = build_old_world(&s);

        apply_validated_save(&mut world, validated_save(&s));

        let buildings = &world.resource::<BuildingRegistry>().definitions;
        assert_eq!(buildings.len(), s.buildings.len());
        assert!(buildings.contains_key(&BuildingType::Farm));
        assert_eq!(buildings.get(&BuildingType::Farm).unwrap().max_level, 5);

        let technologies = &world.resource::<TechnologyRegistry>().definitions;
        assert_eq!(technologies.len(), s.technologies.len());
        assert!(technologies.contains_key("tech_a"));

        let divisions = &world.resource::<MilitaryRegistry>().definitions;
        assert_eq!(divisions.len(), s.divisions.len());
        assert!(divisions.contains_key(&DivisionDefinitionId(0)));

        let stages = &world.resource::<WorldCivilizationState>().stage_definitions;
        assert_eq!(stages.len(), s.world_stages.len());
        assert!(stages.contains_key(&WorldStage::PreIndustrial));
    }

    /// `sync_division_visuals`/`update_frontline_overlay`(division_render.rs/
    /// frontline_render.rs)は`is_changed()`ガードを一切持たず、毎フレーム無条件に現在の
    /// Resourceから全件再構築するため、Resource置換後の次フレームで自動的に追従する
    /// (実コード確認済み、Window/Camera/フォント資産に依存するためこのユニットテストでは
    /// 実行しない)。`update_state_colors_on_controller_change`(map/rendering.rs)と
    /// 上部UIの日付/速度/一時停止表示(ui/top_bar.rs)は`Res<T>::is_changed()`でガードされて
    /// いるため、commit_loadの`World::insert_resource`による置換がその判定を満たすことを
    /// 実際に検証する(本番システムそのものではなく、依存する仕組みを直接検証する)。
    #[test]
    fn resource_replacement_is_observed_as_changed_by_downstream_systems() {
        #[derive(Resource, Default, PartialEq)]
        struct Probe(u32);
        #[derive(Resource, Default)]
        struct ObservedChanges(u32);

        fn observe(probe: Res<Probe>, mut observed: ResMut<ObservedChanges>) {
            if probe.is_changed() {
                observed.0 += 1;
            }
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_once()));
        app.insert_resource(Probe(1));
        app.insert_resource(ObservedChanges::default());
        app.add_systems(Update, observe);

        app.update();
        assert_eq!(
            app.world().resource::<ObservedChanges>().0,
            1,
            "initial insertion must be observed as changed"
        );

        app.update();
        assert_eq!(
            app.world().resource::<ObservedChanges>().0,
            1,
            "an untouched resource must not be observed as changed again"
        );

        // commit_loadと同じ操作(insert_resourceによる置換)を行う。
        app.world_mut().insert_resource(Probe(2));
        app.update();
        assert_eq!(
            app.world().resource::<ObservedChanges>().0,
            2,
            "commit_load's insert_resource-based replacement must be observed as changed on the next system run"
        );
    }

    // ─── AI・一時状態 ────────────────────────────────────────────────────────

    #[test]
    fn ai_normative_state_is_restored() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let cai = world.resource::<CountryAiRegistry>();
        let state = cai.ai_states.get(&CountryId(1)).unwrap();
        assert_eq!(state.mode, CountryAiMode::AtWar);
        assert_eq!(state.cooldown_until_day, 0);
        let mai = world.resource::<MilitaryAiRegistry>();
        let mstate = mai.ai_states.get(&CountryId(1)).unwrap();
        assert_eq!(mstate.estimated_own_power, 4000);
    }

    #[test]
    fn all_four_ai_dirty_flags_are_true_after_apply() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let cai = world.resource::<CountryAiRegistry>();
        assert!(cai.dirty);
        assert!(cai.ai_states.get(&CountryId(1)).unwrap().dirty);
        let mai = world.resource::<MilitaryAiRegistry>();
        assert!(mai.dirty);
        assert!(mai.ai_states.get(&CountryId(1)).unwrap().dirty);
    }

    #[test]
    fn division_army_state_selection_is_cleared() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        assert!(world.resource::<SelectedDivision>().is_empty());
        assert_eq!(world.resource::<SelectedArmy>().0, None);
        assert_eq!(world.resource::<SelectedState>().0, None);
    }

    #[test]
    fn all_panels_are_closed_after_apply() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        assert_eq!(world.resource::<ActivePanel>().current, PanelKind::None);
        assert!(!world.resource::<DiplomacyPanelState>().open);
        assert!(!world.resource::<MilitaryPanelState>().open);
        assert!(!world.resource::<PeacePanelState>().open);
        assert!(!world.resource::<PoliticsPanelState>().open);
        assert!(!world.resource::<ResearchPanelState>().open);
    }

    /// P21-010要求テスト項目25: ロード後にDiplomacyPanelStateの一時的な
    /// 請求州選択・危機開始確認状態がresetされる(commit_loadによる丸ごと置換で
    /// 自動的に満たされることを確認する回帰テスト)。
    #[test]
    fn diplomacy_panel_transient_claim_and_crisis_selection_is_cleared_after_apply() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let diplo = world.resource::<DiplomacyPanelState>();
        assert_eq!(diplo.claim_target_state, None);
        assert_eq!(diplo.pending_crisis_claim, None);
    }

    #[test]
    fn camera_and_drag_state_are_reset() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let camera = world.resource::<CameraDragState>();
        assert_eq!(camera.drag_start, None);
        assert_eq!(camera.camera_start, None);
        let drag = world.resource::<DragSelectState>();
        assert!(!drag.is_dragging);
        assert_eq!(drag.press_start_screen, None);
    }

    #[test]
    fn notification_history_and_pending_ai_war_declarations_are_cleared() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        assert!(world.resource::<NotificationHistory>().recent.is_empty());
        assert!(world.resource::<PendingAiWarDeclarations>().0.is_empty());
    }

    #[test]
    fn save_settings_are_left_untouched_by_apply() {
        // SaveFileConfig等の保存設定はapply.rsが一切触れない(commit_loadのinsert_resource
        // 一覧に含まれない)。LastSaveOutcome/SaveExecutionCountは「直近のセーブ」という
        // ロードとは別の操作についての事実であり、ロードによって無効化される情報ではない
        // ため、意図的にリセットしない(用途調査の結論。詳細は完了報告書を参照)。
        let s = static_data();
        let mut world = build_old_world(&s);
        world.insert_resource(crate::save::runtime::SaveFileConfig::default());
        world.insert_resource(crate::save::runtime::LastSaveOutcome(Some(
            crate::save::write::SaveOutcome::Success {
                path: std::path::PathBuf::from("saves/savegame_v1.ron"),
            },
        )));
        world.insert_resource(crate::save::runtime::SaveExecutionCount(7));

        apply_validated_save(&mut world, validated_save(&s));

        assert_eq!(
            world
                .resource::<crate::save::runtime::SaveExecutionCount>()
                .0,
            7,
            "SaveExecutionCount counts save executions, not loads; apply must not reset it"
        );
        assert!(
            matches!(
                world.resource::<crate::save::runtime::LastSaveOutcome>().0,
                Some(crate::save::write::SaveOutcome::Success { .. })
            ),
            "LastSaveOutcome describes a past save action unrelated to this load; apply must not clear it"
        );
    }

    // ─── 継続性 ──────────────────────────────────────────────────────────────

    #[test]
    fn moving_division_keeps_movement_state_and_can_continue_after_reload() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let division = world
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&DivisionId(7))
            .unwrap();
        assert_eq!(division.status, DivisionStatus::Moving);
        assert_eq!(division.destination, Some(StateId(0)));
        assert_eq!(division.current_path, vec![StateId(0)]);
        assert!((division.movement_progress - 0.35).abs() < f32::EPSILON);
    }

    #[test]
    fn fighting_division_and_battle_keep_combat_state_and_can_continue_after_reload() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let division = world
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&DivisionId(8))
            .unwrap();
        assert_eq!(division.status, DivisionStatus::Fighting);
        assert_eq!(division.combat_id, Some(BattleId(0)));
        let battle = world
            .resource::<BattleRegistry>()
            .battles
            .get(&BattleId(0))
            .unwrap();
        assert_eq!(battle.status, BattleStatus::Ongoing);
        assert!(battle.defender_division_ids.contains(&DivisionId(8)));
    }

    #[test]
    fn new_division_id_after_reload_does_not_collide() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let existing_ids: HashSet<DivisionId> = world
            .resource::<MilitaryRegistry>()
            .divisions
            .keys()
            .copied()
            .collect();

        let template = world
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&DivisionId(7))
            .unwrap()
            .clone();
        let new_id = world
            .resource_mut::<MilitaryRegistry>()
            .add_division(Division {
                id: DivisionId(0),
                ..template
            });

        assert!(
            !existing_ids.contains(&new_id),
            "newly issued DivisionId {new_id:?} must not collide with a restored one"
        );
    }

    #[test]
    fn new_army_id_after_reload_does_not_collide() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));
        let existing_ids: HashSet<ArmyId> = world
            .resource::<ArmyRegistry>()
            .armies
            .keys()
            .copied()
            .collect();

        let template = world
            .resource::<MilitaryRegistry>()
            .divisions
            .get(&DivisionId(7))
            .unwrap()
            .clone();
        let fresh_division_id = world
            .resource_mut::<MilitaryRegistry>()
            .add_division(Division {
                id: DivisionId(0),
                ..template
            });

        let new_army_id = world.resource_scope(|world, mut army_registry: Mut<ArmyRegistry>| {
            let military_registry = world.resource::<MilitaryRegistry>();
            army_registry
                .create_army(CountryId(1), &[fresh_division_id], military_registry)
                .expect("freshly added division must be usable")
        });

        assert!(
            !existing_ids.contains(&new_army_id),
            "newly issued ArmyId {new_army_id:?} must not collide with a restored one"
        );
    }

    #[test]
    fn new_war_and_battle_ids_after_reload_do_not_collide() {
        let s = static_data();
        let mut world = build_old_world(&s);
        apply_validated_save(&mut world, validated_save(&s));

        let existing_war_ids: HashSet<WarId> = world
            .resource::<WarRegistry>()
            .wars
            .keys()
            .copied()
            .collect();
        let existing_battle_ids: HashSet<BattleId> = world
            .resource::<BattleRegistry>()
            .battles
            .keys()
            .copied()
            .collect();

        let new_war_id = world.resource_mut::<WarRegistry>().add_war(War {
            id: WarId(0),
            name: "New War".to_string(),
            attackers: [CountryId(0)].into_iter().collect(),
            defenders: [CountryId(1)].into_iter().collect(),
            war_goals: Vec::new(),
            start_date: "1801/07/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: crate::war::data::WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        });
        assert!(!existing_war_ids.contains(&new_war_id));

        let new_battle_id = world
            .resource_mut::<BattleRegistry>()
            .start_battle(Battle {
                id: BattleId(0),
                war_id: new_war_id,
                state_id: StateId(1),
                attacker_country: CountryId(0),
                defender_country: CountryId(1),
                attacker_division_ids: Vec::new(),
                defender_division_ids: Vec::new(),
                attacker_origins: HashMap::new(),
                start_date: "1801/07/01".to_string(),
                elapsed_days: 0,
                status: BattleStatus::Ongoing,
            })
            .expect("state 1 has no ongoing battle yet");
        assert!(!existing_battle_ids.contains(&new_battle_id));
    }

    #[test]
    fn reloading_after_apply_reproduces_the_original_save_semantically() {
        // ロード直後にexportし直すと、元のDTOと意味的に一致する。GamePaused/AI dirty/
        // UI/カメラはDTOに存在しないため比較対象外(元々SaveGameV1のフィールドではない)。
        let s = static_data();
        let mut world = build_old_world(&s);
        let original_save = representative_save();
        let validated = validate_save_game_v1(original_save.clone(), &context(&s)).unwrap();
        apply_validated_save(&mut world, validated);

        let re_saved = build_save_game_v1(&SaveGameSources {
            game_date: world.resource::<GameDate>(),
            game_speed: world.resource::<GameSpeed>(),
            player_country: world.resource::<PlayerCountry>(),
            world_civilization: world.resource::<WorldCivilizationState>(),
            country_registry: world.resource::<CountryRegistry>(),
            state_registry: world.resource::<StateRegistry>(),
            diplomacy_registry: world.resource::<DiplomacyRegistry>(),
            war_justification_registry: world.resource::<WarJustificationRegistry>(),
            war_registry: world.resource::<WarRegistry>(),
            claim_registry: world.resource::<ClaimRegistry>(),
            crisis_registry: world.resource::<CrisisRegistry>(),
            country_ai_registry: world.resource::<CountryAiRegistry>(),
            military_ai_registry: world.resource::<MilitaryAiRegistry>(),
            military_registry: world.resource::<MilitaryRegistry>(),
            battle_registry: world.resource::<BattleRegistry>(),
            army_registry: world.resource::<ArmyRegistry>(),
            frontline_registry: world.resource::<FrontlineRegistry>(),
        });

        assert_eq!(re_saved.date.year, original_save.date.year);
        assert_eq!(re_saved.date.month, original_save.date.month);
        assert_eq!(re_saved.date.day, original_save.date.day);
        assert!((re_saved.date.accumulator - original_save.date.accumulator).abs() < f64::EPSILON);
        assert_eq!(re_saved.game_speed, original_save.game_speed);
        assert_eq!(re_saved.player_country, original_save.player_country);

        assert_eq!(re_saved.countries.len(), original_save.countries.len());
        for orig_c in &original_save.countries {
            let re_c = re_saved
                .countries
                .iter()
                .find(|c| c.id == orig_c.id)
                .unwrap();
            assert_eq!(re_c.treasury, orig_c.treasury);
            assert_eq!(re_c.government_type, orig_c.government_type);
        }

        assert_eq!(
            re_saved.military.divisions.len(),
            original_save.military.divisions.len()
        );
        assert_eq!(
            re_saved.military.next_division_id,
            original_save.military.next_division_id
        );
        assert_eq!(
            re_saved.armies.armies.len(),
            original_save.armies.armies.len()
        );
        assert_eq!(re_saved.wars.wars.len(), original_save.wars.wars.len());
        assert_eq!(
            re_saved.battles.battles.len(),
            original_save.battles.battles.len()
        );

        let orig_ai = original_save
            .country_ai
            .ai_states
            .get(&CountryId(1))
            .unwrap();
        let re_ai = re_saved.country_ai.ai_states.get(&CountryId(1)).unwrap();
        assert_eq!(re_ai, orig_ai);
    }

    #[test]
    fn applying_the_same_validated_save_to_two_worlds_yields_identical_results() {
        let s = static_data();
        let mut world_a = build_old_world(&s);
        let mut world_b = build_old_world(&s);
        apply_validated_save(&mut world_a, validated_save(&s));
        apply_validated_save(&mut world_b, validated_save(&s));

        assert_eq!(
            world_a.resource::<GameSpeed>().0,
            world_b.resource::<GameSpeed>().0
        );
        assert_eq!(
            world_a.resource::<GameDate>().year,
            world_b.resource::<GameDate>().year
        );
        assert_eq!(
            world_a.resource::<MilitaryRegistry>().divisions.len(),
            world_b.resource::<MilitaryRegistry>().divisions.len()
        );
        let treasuries_a: Vec<f64> = world_a
            .resource::<CountryRegistry>()
            .countries
            .iter()
            .map(|c| c.treasury)
            .collect();
        let treasuries_b: Vec<f64> = world_b
            .resource::<CountryRegistry>()
            .countries
            .iter()
            .map(|c| c.treasury)
            .collect();
        assert_eq!(treasuries_a, treasuries_b);
    }
}
