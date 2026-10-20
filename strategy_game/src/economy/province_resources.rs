//! P32-009: プロヴィンス別の天然資源の入力と、所属ステートへの集計(導出データ)。
//!
//! # 既存の資源値の意味(調査結果。本モジュールはこの意味を変えない)
//! 既存の`StateResourceDeposit`(`economy::resources`)のフィールドは次の意味で使われている:
//! - `base_output`: **採掘施設(Mine)1レベルあたりの月間産出能力**(稼働率100%のとき)。
//!   実産出 = `base_output × Mineのレベル × 稼働率`(`economy::production`)。**埋蔵量でも
//!   採掘済み在庫でもない**。同じステートに同種の鉱床が複数あれば、それぞれが独立に加算される
//!   (産出は線形)。
//! - `discovered`: trueの鉱床だけが産出に使われる(未発見は産出しない)。クリスタル採掘施設の
//!   建設ゲートにも使われる。
//! - `development_level`: 保持されるだけで、現行の計算では参照されない(メタデータ)。
//! - 埋蔵量・枯渇・再生の処理は既存に**存在しない**(値は時間で更新されない)。本モジュールも
//!   追加しない。
//! - 採掘済みの在庫は国家備蓄(`CountryStockpile`)で、地理上の資源値とは別物。産出は生産処理が
//!   備蓄へ加える。資源定義を備蓄へ直接加算することは決してしない。
//! - 資源の分類は既存の`ResourceType`を再利用し、新しい種別を増やさない。
//!
//! # 配置の単位と正本
//! D9に従い、資源の**所在地(入力の正本)はプロヴィンス**(`ProvinceResourcesFile`)。経済処理が
//! 必要とする「ステートで利用できる鉱床」は、所属ステートごとに集計した導出データ
//! (`StateResourcesDerived`)で、保存せず、再生成できる。集計は`base_output`(加算できる産出能力)
//! だけを、同じ`(資源種別, discovered)`ごとに`ProvinceId`昇順で加算する。`development_level`は加算
//! できない属性なので、集計後のステート単位の値は便宜上の最大値(**経済計算に使われない代表値**)とし、
//! プロヴィンスごとの値は`sources`に保持する。
//!
//! # 未設定と資源なしの区別
//! `ProvinceResourcesFile.provinces`に載っているプロヴィンスだけが「設定済み」で、`deposits`が
//! 空なら「資源なし(明示)」。載っていないプロヴィンスは「未設定」であり、資源ゼロへも無制限へも
//! 変換しない。`scope: World`は「地図の全陸プロヴィンス(プロヴィンス表の全件)を設定した」という
//! 宣言で、満たされていなければ入力検証エラー。`scope: Partial`(検証地域など)は全世界の資源配置が
//! 完了したことを意味しない。

use crate::economy::resources::{ResourceType, StateResourceDeposit};
use crate::world::{MapStateId, ProvinceId, WorldModel};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const PROVINCE_RESOURCES_FORMAT_VERSION: u32 = 1;

/// ファイルが設定済みと宣言する範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceScope {
    /// プロヴィンス表の全プロヴィンスを設定済み(省略なし)。
    World,
    /// 一部だけを設定(載っていないプロヴィンスは未設定)。
    Partial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvinceResourceEntry {
    pub province: u32,
    /// このプロヴィンスの鉱床。**空なら「資源なし」を明示**(未設定とは別)。
    pub deposits: Vec<StateResourceDeposit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvinceResourcesFile {
    pub format_version: u32,
    pub scope: ResourceScope,
    pub provinces: Vec<ProvinceResourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceIssue {
    UnsupportedVersion { found: u32 },
    UnknownProvince { province: ProvinceId },
    DuplicateProvince { province: ProvinceId },
    DuplicateResourceInProvince { province: ProvinceId, resource: ResourceType },
    InvalidValue { province: ProvinceId, what: String },
    /// `scope: World`なのに、設定されていないプロヴィンスがある。
    WorldScopeIncomplete { missing: Vec<ProvinceId> },
    /// 集計が有限の値に収まらない(オーバーフロー)。
    NonFiniteTotal { state: MapStateId, resource: ResourceType },
}

impl std::fmt::Display for ResourceIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(
                f,
                "province resources has unsupported format_version {found} (supported: {PROVINCE_RESOURCES_FORMAT_VERSION})"
            ),
            Self::UnknownProvince { province } => write!(
                f,
                "resources refer to unknown {province} (not in the province table)"
            ),
            Self::DuplicateProvince { province } => {
                write!(f, "{province} is configured more than once (ambiguous)")
            }
            Self::DuplicateResourceInProvince { province, resource } => write!(
                f,
                "{province} lists {resource:?} more than once (ambiguous; one entry per resource type)"
            ),
            Self::InvalidValue { province, what } => {
                write!(f, "invalid resource value for {province}: {what}")
            }
            Self::WorldScopeIncomplete { missing } => write!(
                f,
                "scope is World but {} province(s) have no resource setup: {}",
                missing.len(),
                missing.iter().take(10).map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
            ),
            Self::NonFiniteTotal { state, resource } => {
                write!(f, "{state} total for {resource:?} is not finite")
            }
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

/// 読み取り専用のパーサー(入力ファイルを書き換えず、資源を生成しない)。
pub fn parse_province_resources(text: &str) -> Result<ProvinceResourcesFile, String> {
    let v = header_version(text)?;
    if v != PROVINCE_RESOURCES_FORMAT_VERSION {
        return Err(ResourceIssue::UnsupportedVersion { found: v }.to_string());
    }
    ron::from_str(text).map_err(|e| e.to_string())
}

pub fn load_province_resources(path: &str) -> Result<ProvinceResourcesFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?;
    parse_province_resources(&text)
}

/// ステートの資源設定の状況。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateResourceSetup {
    /// 所属する全プロヴィンスが設定済み(資源なしの明示を含む)。
    Complete,
    /// 一部が設定済みで、未設定のプロヴィンスが残る。
    Incomplete,
    /// どのプロヴィンスも設定されていない。
    Unconfigured,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSource {
    pub province: ProvinceId,
    pub deposit: StateResourceDeposit,
}

/// 1ステート分の導出結果。
#[derive(Debug, Clone, PartialEq)]
pub struct StateResourceSummary {
    pub state: MapStateId,
    /// 経済処理へ渡す、ステートで利用できる鉱床(`(資源種別, discovered)`ごとに`base_output`を
    /// プロヴィンスID昇順で加算した結果。順序は`ResourceType::ALL`、discovered=trueが先)。
    pub deposits: Vec<StateResourceDeposit>,
    /// 出典: 各プロヴィンスの鉱床(ProvinceId昇順、プロヴィンス内はファイル順)。
    pub sources: Vec<ResourceSource>,
    pub setup: StateResourceSetup,
    /// 所属プロヴィンスのうち設定済みの数(資源なしの明示を含む)。
    pub configured_provinces: usize,
    /// 設定済みのうち「資源なし」を明示したプロヴィンス数。
    pub explicit_none_provinces: usize,
    /// 未設定のプロヴィンス(ProvinceId昇順)。
    pub unset_provinces: Vec<ProvinceId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateResourcesDerived {
    scope: ResourceScope,
    states: Vec<StateResourceSummary>,
    /// 資源が設定されているが、どのステートにも属さないプロヴィンス(架空のステートへは集計しない)。
    unassigned_configured: Vec<ProvinceId>,
    membership_signature: u64,
    resources_signature: u64,
}

fn type_index(r: ResourceType) -> usize {
    ResourceType::ALL.iter().position(|x| *x == r).unwrap()
}

fn fnv(h: &mut u64, v: u64) {
    for b in v.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// 資源定義の内容の署名(入力の並び順に依存しない)。
fn resources_signature(file: &ProvinceResourcesFile) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    fnv(&mut h, file.scope as u64);
    let mut entries: Vec<&ProvinceResourceEntry> = file.provinces.iter().collect();
    entries.sort_by_key(|e| e.province);
    for e in entries {
        fnv(&mut h, 0xFFFF_0000_0000_0000 | e.province as u64);
        let mut deps: Vec<&StateResourceDeposit> = e.deposits.iter().collect();
        deps.sort_by_key(|d| (type_index(d.resource_type), !d.discovered));
        for d in deps {
            fnv(&mut h, type_index(d.resource_type) as u64);
            fnv(&mut h, d.discovered as u64);
            fnv(&mut h, d.base_output.to_bits());
            fnv(&mut h, d.development_level as u64);
        }
    }
    h
}

/// プロヴィンス別の資源定義を検証し、所属ステートごとに集計する。
/// Errorがあれば部分的な集計を作らず`Err`(存在しないプロヴィンスなどを黙って無視しない)。
pub fn derive_state_resources(
    world: &WorldModel,
    file: &ProvinceResourcesFile,
) -> Result<StateResourcesDerived, Vec<ResourceIssue>> {
    let mut issues = Vec::new();
    if file.format_version != PROVINCE_RESOURCES_FORMAT_VERSION {
        issues.push(ResourceIssue::UnsupportedVersion {
            found: file.format_version,
        });
    }

    // --- 入力検証 ---
    let mut by_province: BTreeMap<ProvinceId, &ProvinceResourceEntry> = BTreeMap::new();
    for e in &file.provinces {
        let pid = ProvinceId(e.province);
        if world.province(pid).is_none() {
            issues.push(ResourceIssue::UnknownProvince { province: pid });
            continue;
        }
        if by_province.insert(pid, e).is_some() {
            issues.push(ResourceIssue::DuplicateProvince { province: pid });
            continue;
        }
        let mut seen = HashSet::new();
        for d in &e.deposits {
            if !seen.insert(type_index(d.resource_type)) {
                issues.push(ResourceIssue::DuplicateResourceInProvince {
                    province: pid,
                    resource: d.resource_type,
                });
            }
            if !(d.base_output.is_finite() && d.base_output >= 0.0) {
                issues.push(ResourceIssue::InvalidValue {
                    province: pid,
                    what: format!(
                        "{:?} base_output {} must be finite and >= 0",
                        d.resource_type, d.base_output
                    ),
                });
            }
        }
    }
    if file.scope == ResourceScope::World {
        let missing: Vec<ProvinceId> = world
            .provinces()
            .iter()
            .map(|p| p.id)
            .filter(|id| !by_province.contains_key(id))
            .collect();
        if !missing.is_empty() {
            issues.push(ResourceIssue::WorldScopeIncomplete { missing });
        }
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    // --- 集計(各プロヴィンスはちょうど1回、所属ステートへ) ---
    let mut states = Vec::with_capacity(world.states().len());
    for s in world.states() {
        let mut sources: Vec<ResourceSource> = Vec::new();
        let mut unset: Vec<ProvinceId> = Vec::new();
        let mut explicit_none = 0usize;
        // `MapStateDef::provinces`はID昇順・重複なし(M0で正規化済み)。
        for pid in &s.provinces {
            match by_province.get(pid) {
                None => unset.push(*pid),
                Some(e) if e.deposits.is_empty() => explicit_none += 1,
                Some(e) => {
                    for d in &e.deposits {
                        sources.push(ResourceSource {
                            province: *pid,
                            deposit: d.clone(),
                        });
                    }
                }
            }
        }
        let configured = s.provinces.len() - unset.len();
        // (資源種別, discovered)ごとに base_output を加算(ProvinceId昇順の固定順)。
        let mut merged: BTreeMap<(usize, bool), (ResourceType, f64, u32)> = BTreeMap::new();
        for src in &sources {
            let d = &src.deposit;
            let e = merged
                .entry((type_index(d.resource_type), !d.discovered))
                .or_insert((d.resource_type, 0.0, 0));
            e.1 += d.base_output;
            // development_level は加算できない属性(経済計算でも未使用)。代表値として最大値。
            e.2 = e.2.max(d.development_level);
        }
        let mut deposits = Vec::with_capacity(merged.len());
        for ((_, undiscovered), (res, total, level)) in merged {
            if !total.is_finite() {
                issues.push(ResourceIssue::NonFiniteTotal {
                    state: s.id,
                    resource: res,
                });
                continue;
            }
            deposits.push(StateResourceDeposit {
                resource_type: res,
                base_output: total,
                discovered: !undiscovered,
                development_level: level,
            });
        }
        let setup = if configured == 0 {
            StateResourceSetup::Unconfigured
        } else if unset.is_empty() {
            StateResourceSetup::Complete
        } else {
            StateResourceSetup::Incomplete
        };
        states.push(StateResourceSummary {
            state: s.id,
            deposits,
            sources,
            setup,
            configured_provinces: configured,
            explicit_none_provinces: explicit_none,
            unset_provinces: unset,
        });
    }
    if !issues.is_empty() {
        return Err(issues);
    }

    let assigned: BTreeSet<ProvinceId> = world
        .states()
        .iter()
        .flat_map(|s| s.provinces.iter().copied())
        .collect();
    let unassigned_configured = by_province
        .keys()
        .filter(|p| !assigned.contains(p))
        .copied()
        .collect();

    Ok(StateResourcesDerived {
        scope: file.scope,
        states,
        unassigned_configured,
        membership_signature: crate::world::aggregate::membership_signature(world),
        resources_signature: resources_signature(file),
    })
}

impl StateResourcesDerived {
    pub fn scope(&self) -> ResourceScope {
        self.scope
    }
    pub fn states(&self) -> &[StateResourceSummary] {
        &self.states
    }
    pub fn state(&self, id: MapStateId) -> Option<&StateResourceSummary> {
        self.states.iter().find(|s| s.state == id)
    }
    pub fn unassigned_configured(&self) -> &[ProvinceId] {
        &self.unassigned_configured
    }
    /// この導出結果が、渡された所属データと資源定義に対して最新か(所属や資源定義が変わっていれば
    /// `false`。古い集計を最新として使わない)。
    pub fn is_current_for(&self, world: &WorldModel, file: &ProvinceResourcesFile) -> bool {
        self.membership_signature == crate::world::aggregate::membership_signature(world)
            && self.resources_signature == resources_signature(file)
    }
    /// 全ステートの全プロヴィンスが設定済みか。`scope: Partial`の導出が真になる場合もあるが、
    /// それは「全世界の資源配置が完了した」ことを意味しない(`scope`も確認すること)。
    pub fn all_states_complete(&self) -> bool {
        self.states.iter().all(|s| s.setup == StateResourceSetup::Complete)
    }
}
