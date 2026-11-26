//! P32-004 (M0): ステート・プロヴィンス制のデータ層。
//!
//! 既存の`state`モジュール(旧`StateId`/`StateData`)とは**独立**しており、互いに参照しない。
//! 型は`ProvinceId`/`MapStateId`(旧`StateId`とは別型)。Bevy非依存の純粋なRustで、
//! 既存のシミュレーション・セーブ・マップ描画には接続していない(後続段階)。
//!
//! 構成:
//! - `ids`: 型付きID。 `fingerprint`: 地理データの識別情報(グリッド/座標集合/陸集合/属性/河川)。
//! - `files`: ゲーム側ファイル(プロヴィンス表・ステート配置)の形式とパーサー。
//! - `model`: 検証済み`WorldModel`と検索。 `validate`: 地理データとの照合(制作途中/ゲーム開始用)。
//!
//! 所属関係の正本は**ステート配置ファイルの`provinces`**(ステート→プロヴィンス)のみ。
//! プロヴィンス→ステートの逆引きは読み込み時に導出する索引で、保存しない。

pub mod aggregate;
pub mod battle;
pub mod connectivity;
pub mod division_scenario;
pub mod facilities;
pub mod files;
pub mod fingerprint;
pub mod ids;
pub mod issue;
pub mod legacy_combat;
pub mod legacy_supply;
pub mod measure;
pub mod model;
pub mod occupation;
pub mod movement;
pub mod overlay;
pub mod peace;
pub mod state_control;
pub mod state_reach;
pub mod supply;
pub mod supply_scenario;
pub mod territory;
pub mod validate;
pub mod war_record;
pub mod war_score;

pub use aggregate::{
    AdjacencyLookup, Completeness, DeriveError, StateAdjacency, StateGeoSummary, WorldGeoDerived,
    derive_world_geo,
};
pub use connectivity::{
    CellClass, EdgeInfo, ExplicitPassable, GraphError, GraphScope, GraphSignature, NotApplicableReason, Passability,
    ProvinceCells, ProvinceGraph, Reach, ReachError, Reachability, SourceProblem, UnknownPassableProvinces,
    UnreachableReason, reach,
};
pub use division_scenario::{
    DIVISION_SCENARIO_FORMAT_VERSION, DivisionScenario, DivisionScenarioError, DivisionScenarioFile, DivisionScenarioIssue,
    load_division_scenario, parse_division_scenario, resolve_division_scenario,
};
pub use facilities::{
    FacilityIssue, FacilityScope, LOCATION_FACILITIES_FORMAT_VERSION, LocationFacilitiesFile,
    LocationFacilityDef, LocationFacilityId, LocationFacilityKind, LocationFacilityRegistry,
    ProvinceFacilitySetup, StateFacilitySetup, WaterAdjacency, load_location_facilities,
    parse_location_facilities, water_adjacency,
};
pub use files::{
    Coverage, ProvinceEntry, ProvinceTableFile, StateEntry, StateLayoutFile, WORLD_FORMAT_VERSION,
    WorldDataError, generate_initial_province_table, load_province_table, load_state_layout,
    parse_province_table, parse_state_layout,
};
pub use fingerprint::{GeographyChange, GeographyFingerprint, compute_fingerprint};
pub use ids::{HexCoord, MapStateId, ProvinceId};
pub use issue::{IssueCode, Severity, WorldIssue};
pub use model::{MapStateDef, ProvinceDef, WorldModel};
pub use battle::{CancelError, CancelledBattle, MapBattle, MapBattleId, MapBattleStatus};
pub use supply::{SupplyCountryConfig, SupplyRuntime, SupplyStats};
pub use occupation::{ClaimOutcome, ClaimRejection, ClaimRequest, countries_to_invalidate};
pub use movement::{
    BattleOutcome, ClaimReason, CombatStats, DEFAULT_BASE_DAYS, FinishReason, MIN_SUPPLY_MODIFIER, SupplyFailure, SupplyReport, MapDivision, MapDivisionId, MapDivisionStatus, MapMilitary, MapMilitaryError, MoveEvent,
    MovementParams, OrderOutcome, OrderRejection, ProvinceControl, StopReason, find_land_path, find_path_where,
};
pub use overlay::{
    BoundaryEdge, BoundaryKind, CellStatus, WorldPreviewData, cell_status, compute_boundaries,
    load_world_preview, state_color,
};
pub use state_control::{
    ControlSummaryError, ShareOf, StateControl, StateControlKind, StateControlSummary, control_kind_label, summarize_control, summarize_control_membership,
};
pub use state_reach::{
    ReachCounts, StateReach, StateReachKind, StateReachSummary, SummaryError, kind_label, summarize_by_state, summarize_membership,
    summarize_model,
};
pub use supply_scenario::{
    AgentOutcome, AgentRef, EvaluateError, SUPPLY_SCENARIO_FORMAT_VERSION, SupplyAgent, SupplyScenario, SupplyScenarioError,
    SupplyScenarioFile, SupplyScenarioIssue, evaluate_supply_scenario, load_supply_scenario, parse_supply_scenario,
    resolve_supply_scenario,
};
pub use territory::{
    ApplyError, CapitalMove, ControlChange, CountryStatus, DisbandReason, Disbanded, EliminationOutcome, Relocation, StaleReason, Territory, TerritoryError, TransferContext, TransferError,
    TransferOutcome, TransferPlan, TransferRequest, apply_transfer, plan_transfer, transfer_state,
};
pub use validate::{ValidationMode, ValidationReport, validate_against_geography};
pub use war_record::{
    ConflictError, Conflicts, EndBasis, EndReason, EndWarApplyError, EndWarError, EndWarOutcome, EndWarPlan, EndWarRequest, EndWarStale, EndedWar, MapWarId, PeaceTerms, SupporterRemoved, TRUCE_DAYS, TruceSet, WarEnd,
    WarRecord, WarResult, WarSide, apply_end_war, end_war, plan_end_war,
};
pub use war_score::{
    AttackerReason, BattleLedger, DefenderReason, LedgerEntry, OccupationRate, Ratio, ScoreError, SkipReason, StateOccupation, Verdict, WarAssessment, WarScoreBreakdown, assess_active_wars, assess_war, assess_with_summary, score_with_summary,
    has_combat_ready_divisions, occupation_rate, war_score,
};
pub use peace::{
    Acceptance, AutoPeaceEntry, AutoPeaceReport, CESSION_MIN_SCORE, PeaceError, PeaceProposal, ProposalTerms, WHITE_PEACE_MIN_SCORE, WHITE_PEACE_MIN_WAR_DAYS, accept_peace, check_acceptance, propose_peace,
    run_auto_peace,
};
