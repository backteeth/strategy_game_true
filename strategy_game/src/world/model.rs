//! 検証済みの`WorldModel`。ファイル同士だけで判定できる構造の検証(重複・参照・二重所属)を
//! 行ってから構築する。地理データとの照合は`validate`側。
//!
//! 内部表現は常に正規化(ID昇順)されるため、入力配列の並び順が違っても、同じ内容なら
//! 同じ`WorldModel`(`PartialEq`)になる。逆引き索引は構築時に導出する(保存しない・二重管理しない)。

use super::files::{Coverage, ProvinceTableFile, StateLayoutFile};
use super::fingerprint::GeographyFingerprint;
use super::ids::{HexCoord, MapStateId, ProvinceId};
use super::issue::{IssueCode, WorldIssue};
use std::collections::{BTreeSet, HashMap};

/// 陸上プロヴィンス(1陸ヘックス=1プロヴィンス)。地形等は持たない(地理データが正)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceDef {
    pub id: ProvinceId,
    pub cell: HexCoord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapStateDef {
    pub id: MapStateId,
    pub name_key: String,
    /// 所属プロヴィンス(ID昇順)。所属の正本。
    pub provinces: Vec<ProvinceId>,
    pub representative: Option<ProvinceId>,
    pub allow_disconnected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldModel {
    coverage: Coverage,
    geography: GeographyFingerprint,
    next_province_id: u32,
    retired: Vec<ProvinceId>,
    provinces: Vec<ProvinceDef>,
    states: Vec<MapStateDef>,
    // --- 以下は導出索引 ---
    by_id: HashMap<ProvinceId, usize>,
    by_cell: HashMap<HexCoord, usize>,
    state_index: HashMap<MapStateId, usize>,
    state_of: HashMap<ProvinceId, MapStateId>,
}

const SAMPLE: usize = 10;

fn sample<T: std::fmt::Display>(items: &[T]) -> String {
    let shown: Vec<String> = items.iter().take(SAMPLE).map(|x| x.to_string()).collect();
    if items.len() > SAMPLE {
        format!("{} ... (+{} more)", shown.join(", "), items.len() - SAMPLE)
    } else {
        shown.join(", ")
    }
}

impl WorldModel {
    /// 構造を検証して構築する。問題はすべて集めて返す(最初の1件で止めない)。
    pub fn build(
        table: ProvinceTableFile,
        layout: StateLayoutFile,
    ) -> Result<WorldModel, Vec<WorldIssue>> {
        let mut issues: Vec<WorldIssue> = Vec::new();

        // --- プロヴィンス表 ---
        let next = table.next_province_id;
        let mut provinces: Vec<ProvinceDef> = Vec::with_capacity(table.provinces.len());
        let mut by_id: HashMap<ProvinceId, usize> = HashMap::new();
        let mut by_cell: HashMap<HexCoord, usize> = HashMap::new();
        for e in &table.provinces {
            let id = ProvinceId(e.id);
            let cell = HexCoord::new(e.q, e.r);
            if e.id >= next {
                issues.push(WorldIssue::error(
                    IssueCode::ProvinceIdNotBelowNext,
                    format!("{id} at {cell} is not below next_province_id {next} (id reuse guard)"),
                ));
            }
            if let Some(&i) = by_id.get(&id) {
                issues.push(WorldIssue::error(
                    IssueCode::DuplicateProvinceId,
                    format!(
                        "{id} is defined twice: at {} and at {cell}",
                        provinces[i].cell
                    ),
                ));
                continue;
            }
            if let Some(&i) = by_cell.get(&cell) {
                issues.push(WorldIssue::error(
                    IssueCode::DuplicateProvinceCell,
                    format!(
                        "cell {cell} is assigned to both {} and {id}",
                        provinces[i].id
                    ),
                ));
                continue;
            }
            by_id.insert(id, provinces.len());
            by_cell.insert(cell, provinces.len());
            provinces.push(ProvinceDef { id, cell });
        }
        let mut retired: Vec<ProvinceId> = Vec::new();
        for &r in &table.retired {
            let id = ProvinceId(r);
            if r >= next {
                issues.push(WorldIssue::error(
                    IssueCode::RetiredIdNotBelowNext,
                    format!("retired {id} is not below next_province_id {next}"),
                ));
            }
            if by_id.contains_key(&id) {
                issues.push(WorldIssue::error(
                    IssueCode::RetiredIdInUse,
                    format!("{id} is listed as retired but still defined (retired IDs must not be reused)"),
                ));
            }
            retired.push(id);
        }
        retired.sort_unstable();
        retired.dedup();

        // --- ステート配置 ---
        let retired_set: BTreeSet<ProvinceId> = retired.iter().copied().collect();
        let mut states: Vec<MapStateDef> = Vec::with_capacity(layout.states.len());
        let mut state_index: HashMap<MapStateId, usize> = HashMap::new();
        let mut state_of: HashMap<ProvinceId, MapStateId> = HashMap::new();
        for s in &layout.states {
            let sid = MapStateId(s.id);
            if state_index.contains_key(&sid) {
                issues.push(WorldIssue::error(
                    IssueCode::DuplicateStateId,
                    format!(
                        "{sid} (name_key '{}') is defined more than once",
                        s.name_key
                    ),
                ));
                continue;
            }
            if s.provinces.is_empty() {
                issues.push(WorldIssue::error(
                    IssueCode::EmptyState,
                    format!("{sid} ('{}') has no provinces", s.name_key),
                ));
            }
            let mut members: Vec<ProvinceId> = Vec::with_capacity(s.provinces.len());
            for &p in &s.provinces {
                let pid = ProvinceId(p);
                if !by_id.contains_key(&pid) {
                    let hint = if retired_set.contains(&pid) {
                        " (this ID is retired)"
                    } else {
                        ""
                    };
                    issues.push(WorldIssue::error(
                        IssueCode::UnknownProvinceInState,
                        format!("{sid} ('{}') references unknown {pid}{hint}", s.name_key),
                    ));
                    continue;
                }
                if let Some(&other) = state_of.get(&pid) {
                    if other != sid {
                        issues.push(WorldIssue::error(
                            IssueCode::ProvinceInMultipleStates,
                            format!("{pid} belongs to both {other} and {sid}"),
                        ));
                    }
                    // 同一ステート内の重複記載は二重所属ではないが、1回に正規化する。
                    continue;
                }
                state_of.insert(pid, sid);
                members.push(pid);
            }
            members.sort_unstable();
            members.dedup();
            let representative = s.representative.map(ProvinceId);
            if let Some(rep) = representative
                && !members.contains(&rep)
            {
                issues.push(WorldIssue::error(
                    IssueCode::RepresentativeNotMember,
                    format!(
                        "{sid} ('{}') representative {rep} is not one of its provinces",
                        s.name_key
                    ),
                ));
            }
            state_index.insert(sid, states.len());
            states.push(MapStateDef {
                id: sid,
                name_key: s.name_key.clone(),
                provinces: members,
                representative,
                allow_disconnected: s.allow_disconnected,
            });
        }

        if !issues.is_empty() {
            return Err(issues);
        }

        // 正規化(ID昇順)。以後、並び順の違いは内容の違いにならない。
        provinces.sort_unstable_by_key(|p| p.id);
        states.sort_unstable_by_key(|s| s.id);
        let by_id = provinces
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        let by_cell = provinces
            .iter()
            .enumerate()
            .map(|(i, p)| (p.cell, i))
            .collect();
        let state_index = states.iter().enumerate().map(|(i, s)| (s.id, i)).collect();

        Ok(WorldModel {
            coverage: table.coverage,
            geography: table.geography,
            next_province_id: table.next_province_id,
            retired,
            provinces,
            states,
            by_id,
            by_cell,
            state_index,
            state_of,
        })
    }

    pub fn coverage(&self) -> Coverage {
        self.coverage
    }
    pub fn geography(&self) -> &GeographyFingerprint {
        &self.geography
    }
    pub fn next_province_id(&self) -> u32 {
        self.next_province_id
    }
    pub fn retired_ids(&self) -> &[ProvinceId] {
        &self.retired
    }
    pub fn provinces(&self) -> &[ProvinceDef] {
        &self.provinces
    }
    pub fn states(&self) -> &[MapStateDef] {
        &self.states
    }

    /// IDからプロヴィンス。
    pub fn province(&self, id: ProvinceId) -> Option<&ProvinceDef> {
        self.by_id.get(&id).map(|&i| &self.provinces[i])
    }
    /// 座標からプロヴィンス(水域・未登録の陸セルは`None`)。
    pub fn province_at(&self, q: i32, r: i32) -> Option<&ProvinceDef> {
        self.by_cell
            .get(&HexCoord::new(q, r))
            .map(|&i| &self.provinces[i])
    }
    pub fn state(&self, id: MapStateId) -> Option<&MapStateDef> {
        self.state_index.get(&id).map(|&i| &self.states[i])
    }
    /// プロヴィンスの所属ステート(未割当は`None`。ダミーのステートへは置き換えない)。
    pub fn state_of(&self, province: ProvinceId) -> Option<MapStateId> {
        self.state_of.get(&province).copied()
    }
    /// ステートの所属プロヴィンス(ID昇順)。存在しないステートは`None`。
    pub fn provinces_of(&self, state: MapStateId) -> Option<&[ProvinceId]> {
        self.state(state).map(|s| s.provinces.as_slice())
    }
    /// 未割当のプロヴィンス(ID昇順)。
    pub fn unassigned_provinces(&self) -> Vec<ProvinceId> {
        self.provinces
            .iter()
            .map(|p| p.id)
            .filter(|id| !self.state_of.contains_key(id))
            .collect()
    }
    /// 存在しないステートへの参照の検出用(後続の所有データなどが使う)。
    pub fn check_state_ref(&self, id: MapStateId) -> Result<(), WorldIssue> {
        if self.state_index.contains_key(&id) {
            Ok(())
        } else {
            Err(WorldIssue::error(
                IssueCode::UnknownState,
                format!("reference to unknown {id}"),
            ))
        }
    }
}

pub(super) fn sample_list<T: std::fmt::Display>(items: &[T]) -> String {
    sample(items)
}
