//! P32-041 (M7-a): ステート単位の領土移転(事前検証 + 一括適用)と国家消滅(純粋な Rust。Bevy 非依存)。
//!
//! **シナリオの所有・支配・首都・交戦関係は検証入力であり、正式な世界データではない。** 範囲は移転・消滅・戦闘中止・師団の帰還・補給の整合まで。戦争の記録・休戦・戦争点数・降伏・経済の帰属・保存は後続の単位(M7-b 以降、M8)。
//!
//! ## 状態
//! [`Territory`] が、実行時の**法的所有**(ステート → 国)、国家ごとの**首都の正本**(`ProvinceId`)、国家の**存続状態**(存続 / 消滅)を持つ。ステートの所属は `WorldModel` を参照する(旧 `StateId` と数値で対応付けない)。
//! 補給の首都は `Territory` の首都と**同じ値**(`check_consistency` で確認。首都が変わるときは同じ値で `MapMilitary::set_capital` を呼ぶ)。補給の設定がない国家にも首都を持たせられる。
//!
//! ## 事前検証と一括適用(旧 `state::ownership` の 2 段階と同じ作法)
//! 1. `plan_transfer`: 状態を**変更しない**(作業用の複製に同じ処理を行い、結果を [`TransferPlan`] にする)。不正な要求は [`TransferError`] で拒否する。
//! 2. `apply_transfer`: 適用の**直前に、現在の状態で同じ検証をやり直し**(所有・支配・首都・師団・戦闘・戦争・補給)、計画の結果と一致したときだけ適用する。古い計画・適用済みの計画は拒否(何も変えない)。
//!    検証はすべて変更の前に済ませ、適用の本体は途中で失敗しない作りにしてある(全か無か)。
//!
//! 日次処理(`advance_day`)の**間**に呼ぶ。日の途中には割り込まない(呼び出しは同期で、日付は進めない)。
//!
//! ## 規則(採用。D5 の第一案の実装。未承認の仕様を確定したものではない)
//! - **移転**: 対象ステートの法的所有を新所有国へ移し、所属する**全プロヴィンスの実効支配を新所有国にそろえる**(旧 `transfer_region_ownership_unchecked` と同じ。第三国による占領も解消し、記録する)。
//! - **首都**: 首都がこのステートにあり、元の所有国に法的所有の陸ステートが残るなら、残るステートを `MapStateId` 昇順に調べ、最初に「元の所有国が支配する有効なプロヴィンス」がある
//!   ステートの代表地点へ移す。代表地点は、ステートの `representative`(元の所有国の支配下なら)、なければ元の所有国が支配する**最小の `ProvinceId`**。候補が無ければ [`TransferError::NoCapitalCandidate`] で拒否する。
//! - **国家消滅**: 最後の法的所有ステートを失った国は消滅(`CountryStatus::Eliminated`。ID は保持し、削除しない)。首都は持たなくなり、師団は解散、その国が当事国の交戦関係と補給の設定は取り除く。
//!   その国が参加する進行中の戦闘は(勝敗・占領なしで)中止する。移転地以外でその国が支配していたプロヴィンスは、そのステートの法的所有国(存続していれば)の支配へ戻す(無ければ支配国なし)。
//! - **戦闘の中止**: 要求の `cancel_battles` に**明示した戦闘**だけを中止する(勝敗・占領なし、参加師団は待機)。移転ステート内に進行中の戦闘があるのに明示がなければ拒否(消滅国の戦闘は自動で中止)。無関係な戦闘は維持する。
//! - **師団の帰還**: 移転ステート内にいる師団のうち、所属国が新所有国でなく、かつ**新所有国と交戦していない**国の師団は帰還させる(別の戦争で敵領内にいる師団を除外する旧 `peace.rs` の規則に相当。M7-a では戦争を終結させないので、
//!   交戦中の師団はそのまま残す。戦争の終結に伴う帰還は M7-b 以降が同じ規則に交戦の条件を加えて呼ぶ)。帰還先は **隣接する自国支配のプロヴィンス(`ProvinceId` 昇順)→ 自国支配の最小 `ProvinceId`**
//!   (どちらも進行中の戦闘がない地点)。帰還先がなければ**解散**(戦闘による撃破とは別の `DisbandReason::NoReturnProvince`)。帰還した師団は経路・目的地を破棄して待機にする。
//!   それ以外の移動命令は触らず、次の `advance_day` の経路検証が再判定する。
//! - **補給の無効化**: 実効支配が変わったプロヴィンスの変更前と変更後の国(第三国を含む)、首都が移った国、消滅した国の到達結果を無効化する。次の補給計画で新しい首都と支配から再計算される。
//!
//! 限界: 帰還の対象は「移転したステート内の師団」だけ(消滅国の旧占領地を所有国へ戻したプロヴィンスにいる他国の師団は対象外)。移転と同時に戦争の終結・休戦・戦争点数は扱わない。

use super::battle::{CancelledBattle, MapBattleId, MapBattleStatus};
use super::connectivity::ProvinceGraph;
use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use super::movement::{MapDivisionId, MapDivisionStatus, MapMilitary};
use crate::common::CountryId;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------- 状態

/// 国家の存続状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountryStatus {
    Active,
    /// 消滅(ID は保持する)。`annexed_by` は最後のステートを受け取った国。
    Eliminated { annexed_by: CountryId },
}

/// 実行時の領土の状態(法的所有・首都の正本・国家の存続)。実効支配は `MapMilitary` が持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct Territory {
    owners: BTreeMap<MapStateId, CountryId>,
    capitals: BTreeMap<usize, ProvinceId>,
    status: BTreeMap<usize, CountryStatus>,
    revision: u64,
    last_applied: Option<TransferRequest>,
}

/// `Territory` を作れない入力。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerritoryError {
    UnknownState(MapStateId),
    UnknownCountry(CountryId),
    UnknownCapitalProvince { country: CountryId, province: ProvinceId },
}

impl std::fmt::Display for TerritoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TerritoryError::UnknownState(s) => write!(f, "法的所有の対象 {s} は世界にありません"),
            TerritoryError::UnknownCountry(c) => write!(f, "CountryId {} は国家の一覧にありません", c.0),
            TerritoryError::UnknownCapitalProvince { country, province } => write!(f, "CountryId {} の首都 {province} は地図にありません", country.0),
        }
    }
}

impl std::error::Error for TerritoryError {}

impl Territory {
    /// 法的所有・首都・国家の一覧から作る(入力の順序に依存しない)。ステートは `model` に、首都は `graph` に存在し、国家は一覧にあること。
    pub fn new(
        model: &WorldModel,
        graph: &ProvinceGraph,
        owners: impl IntoIterator<Item = (MapStateId, CountryId)>,
        capitals: impl IntoIterator<Item = (CountryId, ProvinceId)>,
        countries: impl IntoIterator<Item = CountryId>,
    ) -> Result<Territory, TerritoryError> {
        let status: BTreeMap<usize, CountryStatus> = countries.into_iter().map(|c| (c.0, CountryStatus::Active)).collect();
        let mut o = BTreeMap::new();
        for (s, c) in owners {
            if model.state(s).is_none() {
                return Err(TerritoryError::UnknownState(s));
            }
            if !status.contains_key(&c.0) {
                return Err(TerritoryError::UnknownCountry(c));
            }
            o.insert(s, c);
        }
        let mut caps = BTreeMap::new();
        for (c, p) in capitals {
            if !status.contains_key(&c.0) {
                return Err(TerritoryError::UnknownCountry(c));
            }
            if !graph.contains(p) {
                return Err(TerritoryError::UnknownCapitalProvince { country: c, province: p });
            }
            caps.insert(c.0, p);
        }
        Ok(Territory { owners: o, capitals: caps, status, revision: 0, last_applied: None })
    }

    /// 検証用シナリオの `state_owners`・国家の対応表・補給の首都から作る。
    pub fn from_scenario(sc: &super::division_scenario::DivisionScenario, model: &WorldModel, graph: &ProvinceGraph) -> Result<Territory, TerritoryError> {
        Territory::new(model, graph, sc.state_owners.iter().map(|(s, c)| (*s, *c)), sc.supply.iter().map(|s| (s.country, s.capital)), sc.countries.values().copied())
    }

    /// ステートの法的所有国。
    pub fn owner(&self, state: MapStateId) -> Option<CountryId> {
        self.owners.get(&state).copied()
    }

    /// 法的所有(`MapStateId` 昇順)。
    pub fn owners(&self) -> impl Iterator<Item = (MapStateId, CountryId)> + '_ {
        self.owners.iter().map(|(s, c)| (*s, *c))
    }

    /// 国家が法的所有するステート(`MapStateId` 昇順)。
    pub fn owned_states(&self, country: CountryId) -> Vec<MapStateId> {
        self.owners.iter().filter(|(_, c)| **c == country).map(|(s, _)| *s).collect()
    }

    /// 国家の首都(正本)。
    pub fn capital(&self, country: CountryId) -> Option<ProvinceId> {
        self.capitals.get(&country.0).copied()
    }

    /// 国家の存続状態(一覧にない国家は `None`)。
    pub fn status(&self, country: CountryId) -> Option<CountryStatus> {
        self.status.get(&country.0).copied()
    }

    pub fn is_active(&self, country: CountryId) -> bool {
        self.status(country) == Some(CountryStatus::Active)
    }

    /// 国家の一覧(消滅した国家を含む。`CountryId` 昇順)。
    pub fn countries(&self) -> impl Iterator<Item = CountryId> + '_ {
        self.status.keys().map(|c| CountryId(*c))
    }

    /// 適用した移転の回数(計画が古いかの判定に使う)。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 補給の首都が `Territory` の首都(正本)と同じ値か。違う国家があれば最初の 1 つを理由にして `Err`。
    pub fn check_consistency(&self, military: &MapMilitary) -> Result<(), String> {
        for (c, p) in &self.capitals {
            if let Some(sp) = military.supply_capital(CountryId(*c))
                && sp != *p
            {
                return Err(format!("CountryId {c}: 補給の首都 {sp} が、正本の首都 {p} と違います"));
            }
        }
        for s in self.status.iter().filter(|(_, st)| matches!(st, CountryStatus::Eliminated { .. })).map(|(c, _)| *c) {
            if military.supply_capital(CountryId(s)).is_some() {
                return Err(format!("消滅した CountryId {s} の補給の設定が残っています"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 要求・拒否・結果

/// 移転の要求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    pub state: MapStateId,
    /// 期待する現在の法的所有国(違えば拒否)。
    pub expected_owner: CountryId,
    pub new_owner: CountryId,
    /// 中止する進行中の戦闘(**明示**。他は維持する)。
    pub cancel_battles: Vec<MapBattleId>,
}

impl TransferRequest {
    pub fn new(state: MapStateId, expected_owner: CountryId, new_owner: CountryId) -> TransferRequest {
        TransferRequest { state, expected_owner, new_owner, cancel_battles: Vec::new() }
    }

    pub fn with_cancel_battles(mut self, ids: impl IntoIterator<Item = MapBattleId>) -> TransferRequest {
        self.cancel_battles = ids.into_iter().collect();
        self
    }

    fn normalized(&self) -> TransferRequest {
        let mut r = self.clone();
        r.cancel_battles.sort();
        r.cancel_battles.dedup();
        r
    }
}

/// 要求の拒否(何も変えない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    UnknownState(MapStateId),
    StateWithoutProvinces(MapStateId),
    UnknownCountry(CountryId),
    CountryEliminated(CountryId),
    /// 現在の法的所有国が期待と違う(`actual` が `None` は所有国なし)。
    OwnerMismatch { state: MapStateId, expected: CountryId, actual: Option<CountryId> },
    SameOwner(CountryId),
    UnknownBattle(MapBattleId),
    BattleNotOngoing(MapBattleId),
    /// 移転ステート内に進行中の戦闘があるが、中止の対象に明示されていない。
    BattleInTransferredState { battle: MapBattleId, province: ProvinceId },
    /// 首都を失うが、残る法的所有のステートに首都にできる(元の所有国が支配する)プロヴィンスがない。
    NoCapitalCandidate { country: CountryId },
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferError::UnknownState(s) => write!(f, "{s} は世界にありません"),
            TransferError::StateWithoutProvinces(s) => write!(f, "{s} には所属プロヴィンスがありません"),
            TransferError::UnknownCountry(c) => write!(f, "CountryId {} は国家の一覧にありません", c.0),
            TransferError::CountryEliminated(c) => write!(f, "CountryId {} は消滅しています", c.0),
            TransferError::OwnerMismatch { state, expected, actual } => match actual {
                Some(a) => write!(f, "{state} の法的所有国は CountryId {} で、期待した CountryId {} ではありません", a.0, expected.0),
                None => write!(f, "{state} には法的所有国がなく、期待した CountryId {} ではありません", expected.0),
            },
            TransferError::SameOwner(c) => write!(f, "移転元と移転先が同じ国(CountryId {})です", c.0),
            TransferError::UnknownBattle(b) => write!(f, "{b} は存在しません"),
            TransferError::BattleNotOngoing(b) => write!(f, "{b} は進行中ではありません"),
            TransferError::BattleInTransferredState { battle, province } => write!(f, "移転するステート内の {province} で {battle} が進行中ですが、中止の対象に指定されていません"),
            TransferError::NoCapitalCandidate { country } => write!(f, "CountryId {} が首都を失いますが、残る所有ステートに首都にできる(自国が支配する)プロヴィンスがありません", country.0),
        }
    }
}

impl std::error::Error for TransferError {}

/// 実効支配の変更(`new` が `None` は支配国なし)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlChange {
    pub province: ProvinceId,
    pub previous: Option<CountryId>,
    pub new: Option<CountryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapitalMove {
    pub country: CountryId,
    pub from: ProvinceId,
    pub to: ProvinceId,
}

/// 国家消滅の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EliminationOutcome {
    pub country: CountryId,
    pub annexed_by: CountryId,
    /// 取り除いた交戦関係(`(小さい CountryId, 大きい CountryId)`、昇順)。
    pub wars_removed: Vec<(CountryId, CountryId)>,
    /// 移転地以外で消滅国が支配していたプロヴィンスの戻し先(プロヴィンス昇順)。
    pub control_reverted: Vec<ControlChange>,
    pub supply_removed: bool,
    /// 取り除いた首都(あれば)。
    pub capital_removed: Option<ProvinceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub division: MapDivisionId,
    pub owner: CountryId,
    pub from: ProvinceId,
    pub to: ProvinceId,
}

/// 師団を解散した理由(戦闘による撃破 `MoveEvent::Destroyed` とは別)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisbandReason {
    CountryEliminated,
    NoReturnProvince,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disbanded {
    pub division: MapDivisionId,
    pub owner: CountryId,
    pub at: ProvinceId,
    pub reason: DisbandReason,
}

/// 移転の結果(計画にも適用にも同じ形で出る)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferOutcome {
    pub state: MapStateId,
    pub previous_owner: CountryId,
    pub new_owner: CountryId,
    /// 実効支配の変更(プロヴィンス昇順。移転ステートの全プロヴィンスのうち、支配国が変わったもの)。
    pub control_changes: Vec<ControlChange>,
    /// `control_changes` のうち、解消された**第三国**(元の所有国でも新所有国でもない国)の占領。
    pub third_party_resolved: Vec<ControlChange>,
    pub capital_move: Option<CapitalMove>,
    pub elimination: Option<EliminationOutcome>,
    /// 中止した戦闘(`MapBattleId` 昇順)。
    pub cancelled_battles: Vec<CancelledBattle>,
    /// 帰還した師団(`MapDivisionId` 昇順)。
    pub relocated: Vec<Relocation>,
    /// 解散した師団(消滅国の師団(ID 昇順)、続いて帰還先がない師団(ID 昇順))。
    pub disbanded: Vec<Disbanded>,
    /// 補給の到達結果を無効化した国家(`CountryId` 昇順、重複なし)。
    pub invalidated_countries: Vec<CountryId>,
}

/// 計画(状態を変えずに作った結果)。適用は [`apply_transfer`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPlan {
    request: TransferRequest,
    base_revision: u64,
    outcome: TransferOutcome,
}

impl TransferPlan {
    pub fn request(&self) -> &TransferRequest {
        &self.request
    }
    /// 適用したときの結果(適用の直前に再検証され、一致したときだけ適用される)。
    pub fn outcome(&self) -> &TransferOutcome {
        &self.outcome
    }
}

/// 古い計画の理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleReason {
    /// 計画のあとに別の移転が適用された。
    TerritoryChanged,
    /// 現在の状態では要求自体が成り立たない。
    Invalid(TransferError),
    /// 要求は成り立つが、結果(支配・首都・師団・戦闘・戦争・補給)が計画と違う。
    OutcomeChanged,
}

/// 適用の拒否(何も変えない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// この計画は既に適用済み。
    AlreadyApplied,
    Stale(StaleReason),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::AlreadyApplied => write!(f, "この計画は既に適用済みです"),
            ApplyError::Stale(StaleReason::TerritoryChanged) => write!(f, "計画のあとに別の移転が適用されたため、古い計画です"),
            ApplyError::Stale(StaleReason::Invalid(e)) => write!(f, "現在の状態では成り立たない古い計画です: {e}"),
            ApplyError::Stale(StaleReason::OutcomeChanged) => write!(f, "現在の状態では結果が計画と違うため、古い計画です"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// 計画に必要な参照(世界の所属と地図のグラフ)。
#[derive(Clone, Copy)]
pub struct TransferContext<'a> {
    pub model: &'a WorldModel,
    pub graph: &'a ProvinceGraph,
}

// ---------------------------------------------------------------- 計画・適用

/// 事前検証と計画の作成。`territory`・`military` は変更しない。
pub fn plan_transfer(ctx: TransferContext<'_>, territory: &Territory, military: &MapMilitary, request: &TransferRequest) -> Result<TransferPlan, TransferError> {
    let request = request.normalized();
    let mut t = territory.clone();
    let mut m = military.clone();
    let outcome = execute(ctx, &mut t, &mut m, &request)?;
    Ok(TransferPlan { request, base_revision: territory.revision, outcome })
}

/// 計画を適用する。適用の直前に現在の状態で再検証し、計画と一致しなければ何も変えずに拒否する。
pub fn apply_transfer(ctx: TransferContext<'_>, territory: &mut Territory, military: &mut MapMilitary, plan: &TransferPlan) -> Result<TransferOutcome, ApplyError> {
    if plan.base_revision != territory.revision {
        if territory.revision == plan.base_revision + 1 && territory.last_applied.as_ref() == Some(&plan.request) {
            return Err(ApplyError::AlreadyApplied);
        }
        return Err(ApplyError::Stale(StaleReason::TerritoryChanged));
    }
    // 現在の状態での再検証(複製に実行して、結果が計画と一致するか確かめる)
    {
        let mut t = territory.clone();
        let mut m = military.clone();
        match execute(ctx, &mut t, &mut m, &plan.request) {
            Err(e) => return Err(ApplyError::Stale(StaleReason::Invalid(e))),
            Ok(o) if o != plan.outcome => return Err(ApplyError::Stale(StaleReason::OutcomeChanged)),
            Ok(_) => {}
        }
    }
    // 検証済み: 同じ処理を実体に行う(検証は変更の前に済むので、ここで失敗すれば何も変わっていない)。
    execute(ctx, territory, military, &plan.request).map_err(|e| ApplyError::Stale(StaleReason::Invalid(e)))
}

/// 計画と適用をまとめて行う(間に別の変更を挟まない呼び出し向け)。
pub fn transfer_state(ctx: TransferContext<'_>, territory: &mut Territory, military: &mut MapMilitary, request: &TransferRequest) -> Result<TransferOutcome, ApplyError> {
    let plan = plan_transfer(ctx, territory, military, request).map_err(|e| ApplyError::Stale(StaleReason::Invalid(e)))?;
    apply_transfer(ctx, territory, military, &plan)
}

/// 残る所有ステートの中から、首都にできる代表地点を選ぶ(`MapStateId` 昇順、最初に候補があるステート。`representative` → 自国が支配する最小 `ProvinceId`)。
fn capital_candidate(model: &WorldModel, military: &MapMilitary, country: CountryId, states: &[MapStateId]) -> Option<ProvinceId> {
    let controlled = |p: ProvinceId| military.control().controller(p) == Some(country);
    for s in states {
        let Some(def) = model.state(*s) else { continue };
        if let Some(r) = def.representative
            && def.provinces.contains(&r)
            && controlled(r)
        {
            return Some(r);
        }
        if let Some(p) = def.provinces.iter().copied().filter(|p| controlled(*p)).min() {
            return Some(p);
        }
    }
    None
}

/// 帰還先: 隣接する自国支配のプロヴィンス(昇順)→ 自国支配の最小 `ProvinceId`。どちらも進行中の戦闘がない地点。
fn return_destination(graph: &ProvinceGraph, military: &MapMilitary, owner: CountryId, from: ProvinceId) -> Option<ProvinceId> {
    let eligible = |p: ProvinceId| military.control().controller(p) == Some(owner) && military.ongoing_battle_at(p).is_none();
    graph.neighbor_ids(from).find(|p| eligible(*p)).or_else(|| graph.node_ids().find(|p| eligible(*p)))
}

/// 検証(状態を変えない)→ 変更(途中で失敗しない)。`plan_transfer` は複製に、`apply_transfer` は実体に呼ぶ。
fn execute(ctx: TransferContext<'_>, territory: &mut Territory, military: &mut MapMilitary, req: &TransferRequest) -> Result<TransferOutcome, TransferError> {
    let TransferContext { model, graph } = ctx;
    let (prev, new) = (req.expected_owner, req.new_owner);

    // ---- 検証(ここまで状態を変えない)
    if model.state(req.state).is_none() {
        return Err(TransferError::UnknownState(req.state));
    }
    let provinces: Vec<ProvinceId> = model.provinces_of(req.state).map(|p| p.to_vec()).unwrap_or_default();
    if provinces.is_empty() {
        return Err(TransferError::StateWithoutProvinces(req.state));
    }
    for c in [prev, new] {
        match territory.status(c) {
            None => return Err(TransferError::UnknownCountry(c)),
            Some(CountryStatus::Eliminated { .. }) => return Err(TransferError::CountryEliminated(c)),
            Some(CountryStatus::Active) => {}
        }
    }
    let actual = territory.owner(req.state);
    if actual != Some(prev) {
        return Err(TransferError::OwnerMismatch { state: req.state, expected: prev, actual });
    }
    if prev == new {
        return Err(TransferError::SameOwner(prev));
    }
    let mut cancel: BTreeSet<MapBattleId> = BTreeSet::new();
    for id in &req.cancel_battles {
        match military.battle(*id) {
            None => return Err(TransferError::UnknownBattle(*id)),
            Some(b) if b.status != MapBattleStatus::Ongoing => return Err(TransferError::BattleNotOngoing(*id)),
            Some(_) => {
                cancel.insert(*id);
            }
        }
    }
    let remaining: Vec<MapStateId> = territory.owned_states(prev).into_iter().filter(|s| *s != req.state).collect();
    let eliminated = remaining.is_empty();
    // 消滅国が参加する進行中の戦闘は自動で中止する
    if eliminated {
        for b in military.battles() {
            if b.status == MapBattleStatus::Ongoing && (b.attacker_country == prev || b.defender_country == prev) {
                cancel.insert(b.id);
            }
        }
    }
    for b in military.battles() {
        if b.status == MapBattleStatus::Ongoing && provinces.contains(&b.province) && !cancel.contains(&b.id) {
            return Err(TransferError::BattleInTransferredState { battle: b.id, province: b.province });
        }
    }
    // 首都の再配置
    let mut capital_move: Option<CapitalMove> = None;
    if !eliminated
        && let Some(cap) = territory.capital(prev)
        && model.state_of(cap) == Some(req.state)
    {
        match capital_candidate(model, military, prev, &remaining) {
            Some(to) => capital_move = Some(CapitalMove { country: prev, from: cap, to }),
            None => return Err(TransferError::NoCapitalCandidate { country: prev }),
        }
    }

    // ---- 変更(ここから先は失敗しない)
    let mut invalidate: BTreeSet<usize> = BTreeSet::new();
    let cancelled_battles = military.cancel_battles_unchecked(&cancel);
    territory.owners.insert(req.state, new);
    let mut control_changes: Vec<ControlChange> = Vec::new();
    for p in &provinces {
        let previous = military.control().controller(*p);
        if previous != Some(new) {
            military.control.set(*p, Some(new));
            control_changes.push(ControlChange { province: *p, previous, new: Some(new) });
        }
    }
    let third_party_resolved: Vec<ControlChange> = control_changes.iter().filter(|c| c.previous.is_some_and(|x| x != prev && x != new)).cloned().collect();
    if let Some(m) = &capital_move {
        territory.capitals.insert(m.country.0, m.to);
        military.set_capital(m.country, m.to);
        invalidate.insert(m.country.0);
    }
    let mut disbanded: Vec<Disbanded> = Vec::new();
    let mut elimination: Option<EliminationOutcome> = None;
    if eliminated {
        territory.status.insert(prev.0, CountryStatus::Eliminated { annexed_by: new });
        let capital_removed = territory.capitals.remove(&prev.0);
        let ids: Vec<MapDivisionId> = military.divisions().filter(|d| d.owner == prev).map(|d| d.id).collect();
        for id in ids {
            if let Some(d) = military.divisions.remove(&id) {
                disbanded.push(Disbanded { division: id, owner: prev, at: d.province, reason: DisbandReason::CountryEliminated });
            }
        }
        let wars_removed = military.remove_wars_of(prev);
        let held: Vec<ProvinceId> = military.control().iter().filter(|(_, c)| *c == prev).map(|(p, _)| p).collect();
        let mut control_reverted = Vec::new();
        for p in held {
            let target = model.state_of(p).and_then(|s| territory.owner(s)).filter(|c| territory.is_active(*c));
            military.control.set(p, target);
            control_reverted.push(ControlChange { province: p, previous: Some(prev), new: target });
        }
        let supply_removed = military.remove_supply_country(prev);
        invalidate.insert(prev.0);
        for c in &control_reverted {
            invalidate.extend(c.new.map(|x| x.0));
        }
        elimination = Some(EliminationOutcome { country: prev, annexed_by: new, wars_removed, control_reverted, supply_removed, capital_removed });
    }
    // 師団の帰還(移転ステート内の、所属国が新所有国でなく、新所有国と交戦していない国の師団。ID 昇順)
    let in_state: Vec<MapDivisionId> =
        military.divisions().filter(|d| provinces.contains(&d.province) && d.owner != new && !military.at_war(d.owner, new)).map(|d| d.id).collect();
    let mut relocated: Vec<Relocation> = Vec::new();
    let mut no_return: Vec<Disbanded> = Vec::new();
    for id in in_state {
        let (owner, from) = {
            let d = &military.divisions[&id];
            (d.owner, d.province)
        };
        match return_destination(graph, military, owner, from) {
            Some(to) => {
                let d = military.divisions.get_mut(&id).expect("exists");
                d.province = to;
                d.status = MapDivisionStatus::Idle;
                d.destination = None;
                d.target = None;
                d.path.clear();
                d.progress = 0.0;
                d.battle = None;
                relocated.push(Relocation { division: id, owner, from, to });
            }
            None => {
                military.divisions.remove(&id);
                no_return.push(Disbanded { division: id, owner, at: from, reason: DisbandReason::NoReturnProvince });
            }
        }
    }
    disbanded.extend(no_return);
    // 補給の無効化(実効支配が変わったプロヴィンスの変更前と変更後の国、第三国を含む)
    for c in &control_changes {
        invalidate.extend(c.previous.map(|x| x.0));
        invalidate.extend(c.new.map(|x| x.0));
    }
    let invalidated_countries: Vec<CountryId> = invalidate.into_iter().map(CountryId).collect();
    military.invalidate_reach(&invalidated_countries);
    territory.revision += 1;
    territory.last_applied = Some(req.clone());
    Ok(TransferOutcome {
        state: req.state,
        previous_owner: prev,
        new_owner: new,
        control_changes,
        third_party_resolved,
        capital_move,
        elimination,
        cancelled_battles,
        relocated,
        disbanded,
        invalidated_countries,
    })
}
