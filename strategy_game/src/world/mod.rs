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
pub mod files;
pub mod fingerprint;
pub mod ids;
pub mod issue;
pub mod model;
pub mod overlay;
pub mod validate;

pub use aggregate::{
    AdjacencyLookup, Completeness, DeriveError, StateAdjacency, StateGeoSummary, WorldGeoDerived,
    derive_world_geo,
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
pub use overlay::{
    BoundaryEdge, BoundaryKind, CellStatus, WorldPreviewData, cell_status, compute_boundaries,
    load_world_preview, state_color,
};
pub use validate::{ValidationMode, ValidationReport, validate_against_geography};
