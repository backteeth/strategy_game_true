//! ゲーム側ファイルの形式とパーサー(RON)。制作ツールの地形JSONとは別ファイル。
//!
//! - プロヴィンス表(`ProvinceTableFile`): `ProvinceId`↔陸ヘックス座標と、同期時点の地理識別情報。
//!   地形・気候・植生は**保存しない**(地理JSONが唯一の正)。
//! - ステート配置(`StateLayoutFile`): ステートID・名称キー・所属プロヴィンス(**所属の正本**)。
//!
//! 読み込みは読み取り専用で、IDや所属の自動生成・上書きは行わない。
//! 形式のバージョンは`format_version`。非対応は`WorldDataError::UnsupportedVersion`。

use super::fingerprint::{FINGERPRINT_ALGORITHM, GeographyFingerprint, compute_fingerprint};
use crate::hexmap::data::HexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const WORLD_FORMAT_VERSION: u32 = 1;

/// プロヴィンス表が地図全体の陸セルを網羅しているか。
/// `Partial`は検証用の小地域(その表だけで「全域の所属検証が済んだ」とは判定しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Coverage {
    World,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvinceEntry {
    pub id: u32,
    pub q: i32,
    pub r: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvinceTableFile {
    pub format_version: u32,
    pub coverage: Coverage,
    /// 同期(ID発行)時点の地理識別情報。
    pub geography: GeographyFingerprint,
    /// 次に発行するID。発行済みの全ID(使用中・廃止)より大きい。IDの再利用を防ぐ高水位標。
    pub next_province_id: u32,
    /// 廃止済みID(削除されたセル)。別の土地へ再利用しない。
    #[serde(default)]
    pub retired: Vec<u32>,
    pub provinces: Vec<ProvinceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateEntry {
    pub id: u32,
    /// 表示名の翻訳キー(最小限のメタデータ)。
    pub name_key: String,
    /// 所属プロヴィンスのID。**所属関係の正本**。
    pub provinces: Vec<u32>,
    /// 代表プロヴィンス(任意。指定する場合はこのステートの所属であること)。
    #[serde(default)]
    pub representative: Option<u32>,
    /// 飛び地・島などの非連結を意図した構成であることを明示する(連結性の警告を抑止)。
    #[serde(default)]
    pub allow_disconnected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateLayoutFile {
    pub format_version: u32,
    pub states: Vec<StateEntry>,
}

#[derive(Debug)]
pub enum WorldDataError {
    Io {
        path: String,
        message: String,
    },
    Parse {
        what: &'static str,
        message: String,
    },
    UnsupportedVersion {
        what: &'static str,
        found: u32,
        supported: u32,
    },
}

impl fmt::Display for WorldDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "cannot read '{path}': {message}"),
            Self::Parse { what, message } => write!(f, "{what} is malformed: {message}"),
            Self::UnsupportedVersion {
                what,
                found,
                supported,
            } => write!(
                f,
                "{what} has unsupported format_version {found} (supported: {supported})"
            ),
        }
    }
}

impl std::error::Error for WorldDataError {}

#[derive(Deserialize)]
struct VersionHeader {
    format_version: u32,
}

fn parse_with_version<T: serde::de::DeserializeOwned>(
    what: &'static str,
    text: &str,
) -> Result<T, WorldDataError> {
    // 先にバージョンだけを読み、非対応版はその他のフィールドの形に関わらず明確に失敗させる。
    let header: VersionHeader = ron::from_str(text).map_err(|e| WorldDataError::Parse {
        what,
        message: e.to_string(),
    })?;
    if header.format_version != WORLD_FORMAT_VERSION {
        return Err(WorldDataError::UnsupportedVersion {
            what,
            found: header.format_version,
            supported: WORLD_FORMAT_VERSION,
        });
    }
    ron::from_str(text).map_err(|e| WorldDataError::Parse {
        what,
        message: e.to_string(),
    })
}

pub fn parse_province_table(text: &str) -> Result<ProvinceTableFile, WorldDataError> {
    parse_with_version("province table", text)
}

pub fn parse_state_layout(text: &str) -> Result<StateLayoutFile, WorldDataError> {
    parse_with_version("state layout", text)
}

fn read(path: &str) -> Result<String, WorldDataError> {
    std::fs::read_to_string(path).map_err(|e| WorldDataError::Io {
        path: path.to_string(),
        message: e.to_string(),
    })
}

/// ファイルを読み取り専用で読む(書き込まない)。
pub fn load_province_table(path: &str) -> Result<ProvinceTableFile, WorldDataError> {
    parse_province_table(&read(path)?)
}

pub fn load_state_layout(path: &str) -> Result<StateLayoutFile, WorldDataError> {
    parse_state_layout(&read(path)?)
}

/// **初回の**プロヴィンス表を生成する(明示的に呼ぶ開発用ヘルパー。通常の読み込みからは呼ばれない)。
/// 陸セルを`(r, q)`昇順に並べて0から採番する。以後の追加・削除は後続の同期ツールの責務で、
/// 既存IDの再割り当てや再利用は行わない前提。
pub fn generate_initial_province_table(map: &HexMap, coverage: Coverage) -> ProvinceTableFile {
    let mut land: Vec<(i32, i32)> = map
        .cells()
        .iter()
        .filter(|c| !c.terrain.is_water())
        .map(|c| (c.r, c.q))
        .collect();
    land.sort_unstable();
    let provinces: Vec<ProvinceEntry> = land
        .iter()
        .enumerate()
        .map(|(i, &(r, q))| ProvinceEntry { id: i as u32, q, r })
        .collect();
    let mut geography = compute_fingerprint(map);
    geography.algorithm = FINGERPRINT_ALGORITHM;
    ProvinceTableFile {
        format_version: WORLD_FORMAT_VERSION,
        coverage,
        geography,
        next_province_id: provinces.len() as u32,
        retired: Vec::new(),
        provinces,
    }
}
