//! P32-011: 位置施設(港・要塞・鉄道駅)のプロヴィンス入力層(Bevy 非依存)。
//!
//! D9で承認された方針(位置が重要な施設はプロヴィンス単位)に従い、**所在地を表すデータ層だけ**を
//! 提供する。施設の効果(海上補給・貿易・防御補正・鉄道接続)、建設・費用・維持費・雇用・上限・
//! 建設AIは**一切実装しない**。既存のステート単位の経済施設(`building::data::BuildingType`、
//! 農場・工場など10種)とは別の型で、`BuildingType`へは追加しない(雇用・生産・維持費・建設AIへ
//! 混入しない)。
//!
//! # 正本と導出
//! - **正本は施設定義**(`LocationFacilityDef`: 施設ID・種別・所在地`ProvinceId`)。
//! - プロヴィンス別・種別別の索引(`LocationFacilityRegistry`)は、読み込み時に定義から作る導出データ。
//! - ステート別の施設は**保存せず**、問い合わせのたびに「所在地 → 現在の所属(`WorldModel`)」から
//!   導出する(ステート側に別コピーを持たないため、所属が変わっても古い索引は残らない)。
//!   所属のないプロヴィンスの施設は保持するが、架空のステートへは入れない。
//! - 国家所有・占領者・法的所有は扱わず、推測もしない。
//!
//! # 未設定と施設なし
//! 資源入力(P32-009)と同じ考え方: 施設が1つ以上あるプロヴィンス=設定済み、`confirmed_empty`に載る
//! プロヴィンス=「設定済みで施設なし」(明示)、どちらでもないプロヴィンス=**未設定**(施設ゼロへ
//! 変換しない)。`scope: World`は全プロヴィンスの設定を宣言し、満たさなければ入力エラー。
//! `scope: Partial`(検証地域)は全世界の施設配置の完了を意味しない。
//!
//! # 確定しない事項(維持)
//! 同一プロヴィンスの施設数・同種の重複・スロット数、湖/河川の港の可否、港が接続する海域、
//! 駅の鉄道網、要塞の防御方向、建設条件と上限。ここでは**同一プロヴィンスの複数施設を拒否も
//! 許可の確定もせず、そのまま保持する**(データ層の事実として全件を保持するだけ)。
//! 地理上の隣接(`water_adjacency`)は事実の診断で、建設可否の判定ではない。
//! IDは入力ファイルに明記された値のみを使い、読み込み時に生成しない(配列順で変わらない)。

use super::ids::{HexCoord, MapStateId, ProvinceId};
use super::model::WorldModel;
use crate::hexmap::data::{HexMap, Terrain, hex_neighbors};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LOCATION_FACILITIES_FORMAT_VERSION: u32 = 1;

/// 位置施設の安定ID(入力ファイルに明記された値)。`ProvinceId`・`MapStateId`・国家IDとは別型で、
/// 数値が同じでも互いに変換しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LocationFacilityId(pub u32);

impl std::fmt::Display for LocationFacilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Facility#{}", self.0)
    }
}

/// 位置施設の種別。種別そのものには効果・規模・費用を持たせない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LocationFacilityKind {
    /// 港。
    Port,
    /// 要塞。
    Fortress,
    /// 鉄道駅。
    RailwayStation,
}

impl LocationFacilityKind {
    pub const ALL: [LocationFacilityKind; 3] = [Self::Port, Self::Fortress, Self::RailwayStation];
}

/// ファイルが設定済みと宣言する範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FacilityScope {
    /// プロヴィンス表の全プロヴィンスを設定済み(省略なし)。
    World,
    /// 一部だけを設定(載っていないプロヴィンスは未設定)。
    Partial,
}

/// 施設定義(所在地の正本)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationFacilityDef {
    pub id: u32,
    pub kind: LocationFacilityKind,
    pub province: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationFacilitiesFile {
    pub format_version: u32,
    pub scope: FacilityScope,
    pub facilities: Vec<LocationFacilityDef>,
    /// 「設定済みで施設なし」を明示するプロヴィンス。ここにも施設定義にも載らないプロヴィンスは未設定。
    pub confirmed_empty: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FacilityIssue {
    UnsupportedVersion { found: u32 },
    DuplicateFacilityId { id: LocationFacilityId },
    UnknownProvince { facility: Option<LocationFacilityId>, province: ProvinceId },
    DuplicateConfirmedEmpty { province: ProvinceId },
    /// 「施設なし」と明示されたプロヴィンスに施設が定義されている(矛盾)。
    EmptyButHasFacility { province: ProvinceId, facility: LocationFacilityId },
    /// `scope: World`なのに、設定されていないプロヴィンスがある。
    WorldScopeIncomplete { missing: Vec<ProvinceId> },
}

impl std::fmt::Display for FacilityIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(
                f,
                "location facilities has unsupported format_version {found} (supported: {LOCATION_FACILITIES_FORMAT_VERSION})"
            ),
            Self::DuplicateFacilityId { id } => write!(f, "{id} is defined more than once (ambiguous)"),
            Self::UnknownProvince { facility: Some(id), province } => {
                write!(f, "{id} refers to unknown {province} (not in the province table)")
            }
            Self::UnknownProvince { facility: None, province } => {
                write!(f, "confirmed_empty refers to unknown {province} (not in the province table)")
            }
            Self::DuplicateConfirmedEmpty { province } => {
                write!(f, "{province} is listed in confirmed_empty more than once")
            }
            Self::EmptyButHasFacility { province, facility } => write!(
                f,
                "{province} is confirmed empty but {facility} is located there (contradiction)"
            ),
            Self::WorldScopeIncomplete { missing } => write!(
                f,
                "scope is World but {} province(s) have no facility setup: {}",
                missing.len(),
                missing.iter().take(10).map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

fn header_version(text: &str) -> Result<u32, String> {
    #[derive(Deserialize)]
    struct Header {
        format_version: u32,
    }
    ron::from_str::<Header>(text)
        .map(|h| h.format_version)
        .map_err(|e| e.to_string())
}

/// 読み取り専用のパーサー(入力ファイルを書き換えず、IDや所在地を生成しない)。
pub fn parse_location_facilities(text: &str) -> Result<LocationFacilitiesFile, String> {
    let v = header_version(text)?;
    if v != LOCATION_FACILITIES_FORMAT_VERSION {
        return Err(FacilityIssue::UnsupportedVersion { found: v }.to_string());
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

pub fn load_location_facilities(path: &str) -> Result<LocationFacilitiesFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?;
    parse_location_facilities(&text)
}

/// プロヴィンスの設定状況。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvinceFacilitySetup {
    /// 施設が1つ以上配置されている。
    HasFacilities,
    /// 設定済みで施設なし(明示)。
    ConfirmedEmpty,
    /// 未設定(施設ゼロとは別)。
    Unset,
}

/// ステート内の設定状況。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFacilitySetup {
    /// 所属する全プロヴィンスが設定済み。
    Complete,
    Incomplete,
    Unconfigured,
}

/// 検証済みの位置施設レジストリ。構築に成功したものだけが存在し、エラー時は部分的に採用しない。
/// 読み込み後は不変(所属の変更は`WorldModel`側で、ここは所在地から毎回導出する)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationFacilityRegistry {
    scope: FacilityScope,
    /// 正本: 施設ID昇順。
    facilities: BTreeMap<LocationFacilityId, LocationFacilityDef>,
    /// 導出: 所在地 → 施設ID(昇順)。
    by_province: BTreeMap<ProvinceId, Vec<LocationFacilityId>>,
    confirmed_empty: BTreeSet<ProvinceId>,
}

impl LocationFacilityRegistry {
    /// 検証してレジストリを作る。1件でもエラーがあれば`Err`(部分採用しない)。
    /// `world`はプロヴィンスの存在確認とWorldスコープの網羅確認だけに使い、所属は保存しない。
    pub fn build(
        file: &LocationFacilitiesFile,
        world: &WorldModel,
    ) -> Result<Self, Vec<FacilityIssue>> {
        let mut issues = Vec::new();
        if file.format_version != LOCATION_FACILITIES_FORMAT_VERSION {
            issues.push(FacilityIssue::UnsupportedVersion {
                found: file.format_version,
            });
        }
        let mut facilities = BTreeMap::new();
        let mut by_province: BTreeMap<ProvinceId, Vec<LocationFacilityId>> = BTreeMap::new();
        for d in &file.facilities {
            let id = LocationFacilityId(d.id);
            let pid = ProvinceId(d.province);
            if facilities.insert(id, *d).is_some() {
                issues.push(FacilityIssue::DuplicateFacilityId { id });
                continue;
            }
            if world.province(pid).is_none() {
                issues.push(FacilityIssue::UnknownProvince { facility: Some(id), province: pid });
                continue;
            }
            by_province.entry(pid).or_default().push(id);
        }
        for ids in by_province.values_mut() {
            ids.sort();
        }
        let mut confirmed_empty = BTreeSet::new();
        for p in &file.confirmed_empty {
            let pid = ProvinceId(*p);
            if world.province(pid).is_none() {
                issues.push(FacilityIssue::UnknownProvince { facility: None, province: pid });
                continue;
            }
            if !confirmed_empty.insert(pid) {
                issues.push(FacilityIssue::DuplicateConfirmedEmpty { province: pid });
            }
            if let Some(ids) = by_province.get(&pid) {
                issues.push(FacilityIssue::EmptyButHasFacility { province: pid, facility: ids[0] });
            }
        }
        if file.scope == FacilityScope::World {
            let missing: Vec<ProvinceId> = world
                .provinces()
                .iter()
                .map(|p| p.id)
                .filter(|id| !by_province.contains_key(id) && !confirmed_empty.contains(id))
                .collect();
            if !missing.is_empty() {
                issues.push(FacilityIssue::WorldScopeIncomplete { missing });
            }
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        Ok(Self {
            scope: file.scope,
            facilities,
            by_province,
            confirmed_empty,
        })
    }

    pub fn scope(&self) -> FacilityScope {
        self.scope
    }
    pub fn len(&self) -> usize {
        self.facilities.len()
    }
    pub fn is_empty(&self) -> bool {
        self.facilities.is_empty()
    }

    /// 施設IDから施設定義。
    pub fn facility(&self, id: LocationFacilityId) -> Option<&LocationFacilityDef> {
        self.facilities.get(&id)
    }

    /// 全施設(ID昇順、各1回)。
    pub fn iter(&self) -> impl Iterator<Item = &LocationFacilityDef> {
        self.facilities.values()
    }

    /// 所在地のプロヴィンスにある全施設(ID昇順。同じプロヴィンスの複数施設を欠落させない)。
    pub fn facilities_in_province(&self, province: ProvinceId) -> Vec<&LocationFacilityDef> {
        self.by_province
            .get(&province)
            .map(|ids| ids.iter().map(|i| &self.facilities[i]).collect())
            .unwrap_or_default()
    }

    /// 種別ごとの施設(ID昇順)。
    pub fn facilities_of_kind(&self, kind: LocationFacilityKind) -> Vec<&LocationFacilityDef> {
        self.facilities.values().filter(|d| d.kind == kind).collect()
    }

    /// ステートの所属プロヴィンスにある施設(ID昇順)。ステート側に別コピーは持たず、所在地と
    /// **渡された`world`の現在の所属**から毎回導出する。存在しないステートは`None`。
    pub fn facilities_in_state(
        &self,
        world: &WorldModel,
        state: MapStateId,
    ) -> Option<Vec<&LocationFacilityDef>> {
        let provinces = world.provinces_of(state)?;
        let mut out: Vec<&LocationFacilityDef> = provinces
            .iter()
            .flat_map(|p| self.facilities_in_province(*p))
            .collect();
        out.sort_by_key(|d| d.id);
        Some(out)
    }

    /// どのステートにも属さないプロヴィンスにある施設(架空のステートへは入れない)。
    pub fn unassigned_facilities(&self, world: &WorldModel) -> Vec<&LocationFacilityDef> {
        self.facilities
            .values()
            .filter(|d| world.state_of(ProvinceId(d.province)).is_none())
            .collect()
    }

    /// 施設の所属ステート(所在地の現在の所属から導出。未割当は`None`)。
    pub fn state_of_facility(
        &self,
        world: &WorldModel,
        id: LocationFacilityId,
    ) -> Option<MapStateId> {
        self.facility(id)
            .and_then(|d| world.state_of(ProvinceId(d.province)))
    }

    pub fn province_setup(&self, province: ProvinceId) -> ProvinceFacilitySetup {
        if self.by_province.contains_key(&province) {
            ProvinceFacilitySetup::HasFacilities
        } else if self.confirmed_empty.contains(&province) {
            ProvinceFacilitySetup::ConfirmedEmpty
        } else {
            ProvinceFacilitySetup::Unset
        }
    }

    /// ステートの設定状況(存在しないステートは`None`)。
    pub fn state_setup(&self, world: &WorldModel, state: MapStateId) -> Option<StateFacilitySetup> {
        let provinces = world.provinces_of(state)?;
        let set = provinces
            .iter()
            .filter(|p| self.province_setup(**p) != ProvinceFacilitySetup::Unset)
            .count();
        Some(if set == 0 {
            StateFacilitySetup::Unconfigured
        } else if set == provinces.len() {
            StateFacilitySetup::Complete
        } else {
            StateFacilitySetup::Incomplete
        })
    }
}

/// セルの周囲6近傍の水域の事実(建設可否の判定ではない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterAdjacency {
    pub sea: usize,
    pub lake: usize,
    /// 地図の範囲外、または地図にセルが無い近傍(水域とも陸とも判断しない)。
    pub unmapped: usize,
}

impl WaterAdjacency {
    pub fn touches_sea(&self) -> bool {
        self.sea > 0
    }
    pub fn touches_lake(&self) -> bool {
        self.lake > 0
    }
    pub fn touches_water(&self) -> bool {
        self.sea + self.lake > 0
    }
}

/// 陸セルの6近傍のうち、海・湖であるものを数える。河川は含めない(河港・航行可否は未決)。
/// 海と湖は区別して返す。その結果を港の許可に使うかどうかはゲームルールで、ここでは決めない。
pub fn water_adjacency(map: &HexMap, cell: HexCoord) -> WaterAdjacency {
    let mut w = WaterAdjacency { sea: 0, lake: 0, unmapped: 0 };
    for (q, r) in hex_neighbors(cell.q, cell.r) {
        match map.cell(q, r).map(|c| c.terrain) {
            Some(Terrain::Sea) => w.sea += 1,
            Some(Terrain::Lake) => w.lake += 1,
            Some(_) => {}
            None => w.unmapped += 1,
        }
    }
    w
}
