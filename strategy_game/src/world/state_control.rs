//! P32-038 (M6-e): プロヴィンスの実効支配(`ProvinceControl`)を**ステート単位に集計**する(純粋な Rust。M7 へ渡す集計値)。
//!
//! 入力は ①所属(`WorldModel`: ステート → プロヴィンス。正本)②法的所有(検証シナリオの `state_owners`。**検証入力であり、正式な所有データではない**)
//! ③実効支配(`ProvinceControl`)。集計は毎回**プロヴィンスの支配から導出**し、ステートに独立した支配データは持たせない(保存もしない)。
//! 補給の到達要約(`state_reach`)とは別の用途で、ここでは使わない。
//!
//! ## 規則
//! - ステートごとに、法的所有国・所属プロヴィンス総数(分母)・国別の支配件数・支配国なしの件数を返す。出力は `MapStateId` 昇順、国別は `CountryId` 昇順。
//! - 区分(`StateControlKind`): 所属が 0 件 = `NoProvinces`、全件が支配国なし = `NoController`(所属 0 件とは別)、全件を同じ国が支配 = `Full(国)`、それ以外(複数国・一部に支配国なし・一部だけ支配)= `Mixed`(内訳を見る)。
//! - ステート未所属のプロヴィンスは、どのステートの分母にも入れない(件数だけ `unassigned` に別に持つ)。
//! - 割合は `ShareOf { numerator: その国の支配件数, denominator: ステートの所属総数 }`(支配国なしも分母に含む。比較しやすいよう分子・分母を分けて保持。小数にしない)。
//! - 法的所有国と実効支配は別々の値。**第三国の支配は国別の内訳に含めるが、交戦国による占領には加算しない**。「占領」と呼ぶ場合は、対象の交戦関係(所有国 O と占領国 X が交戦中)を
//!   `occupation_by` の引数の述語で明示して判定する(交戦中でなければ `None`)。
//! - 各ステートで、国別件数の合計 + 支配国なし = 所属総数(`is_consistent`)。入力の順序に依存しない(内部は `BTreeMap`)。

use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use super::movement::ProvinceControl;
use crate::common::CountryId;
use std::collections::{BTreeMap, BTreeSet};

/// 割合(分子 / 分母)。分母は 0 にならない(所属 0 件のステートは割合を作らない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareOf {
    pub numerator: usize,
    pub denominator: usize,
}

impl ShareOf {
    /// 表示用の小数(比較には分子・分母を使うこと)。
    pub fn as_f64(&self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

/// ステートの支配の区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateControlKind {
    /// 所属プロヴィンスが 0 件(分母 0)。
    NoProvinces,
    /// 所属は 1 件以上あり、全件が支配国なし。
    NoController,
    /// 所属の全件を同じ国が支配。
    Full(CountryId),
    /// 複数国が支配している、または一部に支配国なしがある(内訳を見る)。
    Mixed,
}

/// 1 ステートの集計。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateControl {
    pub state: MapStateId,
    /// 法的所有国(検証入力。無ければ `None`)。実効支配とは別の値。
    pub legal_owner: Option<CountryId>,
    /// 所属プロヴィンス総数(分母)。
    pub total: usize,
    /// 国別の支配件数(`CountryId` 昇順。0 件の国は含めない)。
    pub by_country: Vec<(CountryId, usize)>,
    /// 支配国なしの件数。
    pub uncontrolled: usize,
}

impl StateControl {
    pub fn kind(&self) -> StateControlKind {
        if self.total == 0 {
            StateControlKind::NoProvinces
        } else if self.uncontrolled == self.total {
            StateControlKind::NoController
        } else if self.by_country.len() == 1 && self.by_country[0].1 == self.total {
            StateControlKind::Full(self.by_country[0].0)
        } else {
            StateControlKind::Mixed
        }
    }

    /// 国別・支配国なしの合計が所属総数に一致するか。
    pub fn is_consistent(&self) -> bool {
        self.by_country.iter().map(|(_, n)| n).sum::<usize>() + self.uncontrolled == self.total
    }

    /// その国の支配件数(支配していなければ 0)。
    pub fn count_of(&self, country: CountryId) -> usize {
        self.by_country.iter().find(|(c, _)| *c == country).map_or(0, |(_, n)| *n)
    }

    /// その国の支配割合(所属 0 件なら `None`)。
    pub fn share_of(&self, country: CountryId) -> Option<ShareOf> {
        (self.total > 0).then_some(ShareOf { numerator: self.count_of(country), denominator: self.total })
    }

    /// 支配国なしの割合(所属 0 件なら `None`)。
    pub fn uncontrolled_share(&self) -> Option<ShareOf> {
        (self.total > 0).then_some(ShareOf { numerator: self.uncontrolled, denominator: self.total })
    }

    /// 法的所有国の支配割合(法的所有国が無い、または所属 0 件なら `None`)。
    pub fn owner_share(&self) -> Option<ShareOf> {
        self.legal_owner.and_then(|o| self.share_of(o))
    }

    /// `occupier` による**法的所有国への占領**の件数と割合。対象の交戦関係は `at_war(所有国, 占領国)` で明示する。
    /// 法的所有国が無い・占領国が所有国と同じ・交戦中でない場合は `None`(第三国の支配はここに加算されない)。支配件数が 0 でも、交戦中なら `Some`(0 / 総数)。
    pub fn occupation_by(&self, occupier: CountryId, at_war: &dyn Fn(CountryId, CountryId) -> bool) -> Option<ShareOf> {
        let owner = self.legal_owner?;
        if owner == occupier || !at_war(owner, occupier) {
            return None;
        }
        self.share_of(occupier)
    }
}

/// 集計の入力エラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlSummaryError {
    DuplicateState(MapStateId),
    ProvinceInMultipleStates(ProvinceId),
    /// 法的所有の対象が所属表に無いステート。
    OwnerOfUnknownState(MapStateId),
}

impl std::fmt::Display for ControlSummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateState(s) => write!(f, "{s} appears more than once in the membership"),
            Self::ProvinceInMultipleStates(p) => write!(f, "{p} belongs to more than one state"),
            Self::OwnerOfUnknownState(s) => write!(f, "a legal owner is given for {s}, which is not in the membership"),
        }
    }
}

impl std::error::Error for ControlSummaryError {}

/// ステート単位の集計(`MapStateId` 昇順)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateControlSummary {
    states: Vec<StateControl>,
    unassigned: usize,
}

impl StateControlSummary {
    pub fn states(&self) -> &[StateControl] {
        &self.states
    }

    pub fn state(&self, id: MapStateId) -> Option<&StateControl> {
        self.states.binary_search_by_key(&id, |s| s.state).ok().map(|i| &self.states[i])
    }

    /// ステート未所属のプロヴィンスの件数(どのステートの分母にも入っていない)。
    pub fn unassigned(&self) -> usize {
        self.unassigned
    }

    pub fn is_consistent(&self) -> bool {
        self.states.iter().all(StateControl::is_consistent)
    }
}

/// 所属表(ステート → プロヴィンス)・法的所有・実効支配から集計する。
pub fn summarize_control_membership(
    members: &[(MapStateId, &[ProvinceId])],
    unassigned_provinces: &[ProvinceId],
    owners: &BTreeMap<MapStateId, CountryId>,
    control: &ProvinceControl,
) -> Result<StateControlSummary, ControlSummaryError> {
    let mut by_state: BTreeMap<MapStateId, StateControl> = BTreeMap::new();
    let mut seen_provinces: BTreeSet<ProvinceId> = BTreeSet::new();
    for (state, provinces) in members {
        if by_state.contains_key(state) {
            return Err(ControlSummaryError::DuplicateState(*state));
        }
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        let mut uncontrolled = 0usize;
        for p in *provinces {
            if !seen_provinces.insert(*p) {
                return Err(ControlSummaryError::ProvinceInMultipleStates(*p));
            }
            match control.controller(*p) {
                Some(c) => *counts.entry(c.0).or_default() += 1,
                None => uncontrolled += 1,
            }
        }
        by_state.insert(
            *state,
            StateControl { state: *state, legal_owner: owners.get(state).copied(), total: provinces.len(), by_country: counts.into_iter().map(|(c, n)| (CountryId(c), n)).collect(), uncontrolled },
        );
    }
    if let Some(s) = owners.keys().find(|s| !by_state.contains_key(s)) {
        return Err(ControlSummaryError::OwnerOfUnknownState(*s));
    }
    let mut unassigned = 0usize;
    for p in unassigned_provinces {
        if !seen_provinces.insert(*p) {
            return Err(ControlSummaryError::ProvinceInMultipleStates(*p));
        }
        unassigned += 1;
    }
    Ok(StateControlSummary { states: by_state.into_values().collect(), unassigned })
}

/// `WorldModel` の所属(正本)から集計する(`summarize_control_membership` の薄い包み)。
pub fn summarize_control(model: &WorldModel, owners: &BTreeMap<MapStateId, CountryId>, control: &ProvinceControl) -> Result<StateControlSummary, ControlSummaryError> {
    let members: Vec<(MapStateId, &[ProvinceId])> = model.states().iter().map(|s| (s.id, s.provinces.as_slice())).collect();
    summarize_control_membership(&members, &model.unassigned_provinces(), owners, control)
}

/// 区分の表示名(CLI・報告用)。
pub fn control_kind_label(k: StateControlKind) -> &'static str {
    match k {
        StateControlKind::NoProvinces => "所属プロヴィンスなし (no provinces)",
        StateControlKind::NoController => "全件が支配国なし (no controller)",
        StateControlKind::Full(_) => "全域支配 (full)",
        StateControlKind::Mixed => "内訳あり (mixed)",
    }
}
