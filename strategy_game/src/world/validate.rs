//! 地理データ(`HexMap`)とゲーム側データの照合。
//!
//! 2つのモード:
//! - `Authoring`(制作途中): 陸プロヴィンスの未割当や部分的なプロヴィンス表を**診断(Warning/Info)**
//!   として許容する。
//! - `GameStart`(ゲーム開始用): 表が地図全体の陸セルを網羅(`Coverage::World`)し、
//!   すべての陸プロヴィンスがいずれかのステートに所属していなければ**失敗**する。
//!
//! どちらのモードでも、地理的な構造不整合(グリッド変更、陸集合の変更、存在しない/水域のセルを
//! 指すプロヴィンス、陸セルに対応するプロヴィンスの欠落[表が全域を謳う場合])はエラー。
//! 連結性(島・飛び地)は**エラーにせず**、警告として報告する(ルールは未決)。

use super::files::Coverage;
use super::fingerprint::compute_fingerprint;
use super::ids::HexCoord;
use super::issue::{IssueCode, Severity, WorldIssue};
use super::model::{WorldModel, sample_list};
use crate::hexmap::data::{HexMap, hex_neighbors};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    Authoring,
    GameStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    pub mode: ValidationMode,
    pub issues: Vec<WorldIssue>,
    /// 全世界の陸セルについて所属検証が完了したか(`Coverage::World`で、地理との不整合が無い場合のみtrue)。
    /// 小地域の表が読み込めただけではtrueにならない。
    pub world_coverage_verified: bool,
    pub unassigned_province_count: usize,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
    pub fn errors(&self) -> impl Iterator<Item = &WorldIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }
    pub fn has(&self, code: IssueCode) -> bool {
        self.issues.iter().any(|i| i.code == code)
    }
    pub fn severity_of(&self, code: IssueCode) -> Option<Severity> {
        self.issues
            .iter()
            .find(|i| i.code == code)
            .map(|i| i.severity)
    }
}

pub fn validate_against_geography(
    model: &WorldModel,
    map: &HexMap,
    mode: ValidationMode,
) -> ValidationReport {
    let mut issues: Vec<WorldIssue> = Vec::new();

    // --- 1. 地理識別情報の比較(種別ごとに区別) ---
    let change = model.geography().compare(&compute_fingerprint(map));
    if change.algorithm_mismatch {
        issues.push(WorldIssue::error(
            IssueCode::FingerprintAlgorithmMismatch,
            format!(
                "stored geography fingerprint algorithm {} differs from the current one; re-sync is required",
                model.geography().algorithm
            ),
        ));
    }
    if change.grid {
        issues.push(WorldIssue::error(
            IssueCode::GridChanged,
            "grid definition (orientation/size/origin/image size/extent) changed since the last sync; \
             coordinates no longer correspond to the stored province table",
        ));
    }
    if change.land_set {
        issues.push(WorldIssue::error(
            IssueCode::LandSetChanged,
            "the set of land cells changed since the last sync (land cells were added/removed, or cells \
             changed between land and water); review the per-cell findings below and re-sync explicitly",
        ));
    }
    if change.coordinates && !change.land_set {
        issues.push(WorldIssue::new(
            Severity::Warning,
            IssueCode::CoordinatesChanged,
            "water cell coordinates were added/removed since the last sync; provinces (land) are unaffected",
        ));
    }
    if change.attributes && !change.land_set {
        issues.push(WorldIssue::new(
            Severity::Info,
            IssueCode::AttributesChanged,
            "terrain/climate/vegetation attributes changed since the last sync; province IDs and \
             state membership are unaffected",
        ));
    }
    if change.rivers {
        issues.push(WorldIssue::new(
            Severity::Info,
            IssueCode::RiversChanged,
            "river edges changed since the last sync; province IDs and state membership are unaffected",
        ));
    }

    // --- 2. 各プロヴィンスのセル ---
    let mut missing: Vec<String> = Vec::new();
    let mut water: Vec<String> = Vec::new();
    for p in model.provinces() {
        match map.cell(p.cell.q, p.cell.r) {
            None => missing.push(format!("{} at {}", p.id, p.cell)),
            Some(c) if c.terrain.is_water() => {
                water.push(format!("{} at {} ({})", p.id, p.cell, c.terrain.id()))
            }
            Some(_) => {}
        }
    }
    if !missing.is_empty() {
        issues.push(WorldIssue::error(
            IssueCode::ProvinceCellMissing,
            format!(
                "{} province(s) refer to cells that no longer exist in the map (cells removed or coordinates shifted): {}",
                missing.len(),
                sample_list(&missing)
            ),
        ));
    }
    if !water.is_empty() {
        issues.push(WorldIssue::error(
            IssueCode::ProvinceCellIsWater,
            format!(
                "{} province(s) are placed on water cells (provinces must be land): {}",
                water.len(),
                sample_list(&water)
            ),
        ));
    }

    // --- 3. 陸セルに対応するプロヴィンスの欠落 ---
    let mut uncovered: Vec<HexCoord> = map
        .cells()
        .iter()
        .filter(|c| !c.terrain.is_water() && model.province_at(c.q, c.r).is_none())
        .map(|c| HexCoord::new(c.q, c.r))
        .collect();
    uncovered.sort_unstable_by_key(|c| (c.r, c.q));
    if !uncovered.is_empty() {
        let severity = if model.coverage() == Coverage::World {
            Severity::Error
        } else {
            Severity::Warning
        };
        issues.push(WorldIssue::new(
            severity,
            IssueCode::LandCellWithoutProvince,
            format!(
                "{} land cell(s) have no province: {}",
                uncovered.len(),
                sample_list(&uncovered)
            ),
        ));
    }
    if model.coverage() == Coverage::Partial {
        let (sev, tail) = match mode {
            ValidationMode::GameStart => (
                Severity::Error,
                "a partial province table cannot start a game",
            ),
            ValidationMode::Authoring => (
                Severity::Info,
                "validation of the whole world is NOT complete",
            ),
        };
        issues.push(WorldIssue::new(
            sev,
            IssueCode::PartialCoverage,
            format!("province table covers only part of the map: {tail}"),
        ));
    }

    // --- 4. 未割当の陸プロヴィンス ---
    let unassigned = model.unassigned_provinces();
    if !unassigned.is_empty() {
        let severity = match mode {
            ValidationMode::Authoring => Severity::Warning,
            ValidationMode::GameStart => Severity::Error,
        };
        issues.push(WorldIssue::new(
            severity,
            IssueCode::UnassignedProvince,
            format!(
                "{} land province(s) belong to no state: {}",
                unassigned.len(),
                sample_list(&unassigned)
            ),
        ));
    }

    // --- 5. ステートの連結性(警告のみ。島・飛び地は禁止しない) ---
    for s in model.states() {
        if s.allow_disconnected || s.provinces.len() < 2 {
            continue;
        }
        let comps = connected_components(model, &s.provinces);
        if comps > 1 {
            issues.push(WorldIssue::new(
                Severity::Warning,
                IssueCode::StateDisconnected,
                format!(
                    "{} ('{}') splits into {comps} disconnected parts (islands/exclaves are allowed; set \
                     allow_disconnected to acknowledge)",
                    s.id, s.name_key
                ),
            ));
        }
    }

    let has_geo_error = issues.iter().any(|i| {
        i.severity == Severity::Error
            && !matches!(
                i.code,
                IssueCode::UnassignedProvince | IssueCode::PartialCoverage
            )
    });
    ValidationReport {
        mode,
        world_coverage_verified: model.coverage() == Coverage::World && !has_geo_error,
        unassigned_province_count: unassigned.len(),
        issues,
    }
}

/// ステートに属するプロヴィンスの、6近傍による連結成分数(陸セルの隣接のみで数える)。
fn connected_components(model: &WorldModel, members: &[super::ids::ProvinceId]) -> usize {
    let cells: HashSet<HexCoord> = members
        .iter()
        .filter_map(|id| model.province(*id).map(|p| p.cell))
        .collect();
    let mut seen: HashSet<HexCoord> = HashSet::with_capacity(cells.len());
    let mut comps = 0;
    for &start in &cells {
        if !seen.insert(start) {
            continue;
        }
        comps += 1;
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            for (nq, nr) in hex_neighbors(c.q, c.r) {
                let n = HexCoord::new(nq, nr);
                if cells.contains(&n) && seen.insert(n) {
                    stack.push(n);
                }
            }
        }
    }
    comps
}
