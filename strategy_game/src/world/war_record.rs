//! P32-042 (M7-b): 戦争記録・終結・休戦(純粋な Rust。Bevy 非依存)。
//!
//! **シナリオの戦争・所有・支配は検証入力であり、正式な世界データではない。** 範囲は戦争の記録、日次処理の間に呼ぶ終結 API(白紙講和とステート割譲)、
//! 同一目標の別戦争の失効、国家消滅に伴う他戦争の決着、休戦の記録まで。戦争点数・降伏・講和の受諾 UI・経済・保存は後続の単位。
//!
//! ## 記録
//! [`WarRecord`]: 戦争 ID・両陣営(代表国 1 + 支持国)・対象 `MapStateId` 1 件・開始日・終結(日・結果・理由)。日は呼び出し側の日番号(`advance_day` の回数に合わせる)。
//! 移動・戦闘が参照する交戦関係(`MapMilitary::at_war`)は、**稼働中の戦争の両陣営の全組み合わせ**と、記録を持たない従来の交戦関係(シナリオの `wars`。以下「従来の組」)の和集合から導出する
//! ([`Conflicts::sync`])。記録のないシナリオは従来どおり(従来の組だけ)で動く。同じ組を記録と従来の組の両方に書くことはできない。
//!
//! ## 終結 API([`plan_end_war`] → [`apply_end_war`])
//! 事前検証は状態を変えない(作業用の複製に同じ処理を行う)。適用は直前に現在の状態で再検証し、結果が計画と一致したときだけ行う。
//! 終結済みの戦争への再要求は [`EndWarError::AlreadyEnded`] / [`EndWarApplyError::AlreadyEnded`] で拒否し、割譲・休戦は二重に適用されない。
//! 日次処理(`advance_day`)の**間**に呼ぶ。
//!
//! 順序(失敗し得る検証はすべて変更の前):
//! 1. 要求の検証(戦争の存在・稼働中)。割譲なら M7-a の `world::territory::execute`(計画・検証・適用の本体)を再利用する(失敗しても何も変えない)。
//! 2. 終結の連鎖を作業用の記録に対して決める。割譲で消滅する国が代表国の別の戦争は**対陣営の勝利**(理由 `AttackerEliminated`/`DefenderEliminated`)、
//!    支持国としてのみ参加なら**陣営から除く**。割譲したステートを目標とする残りの稼働中の戦争は**白紙講和**で失効(理由 `TargetOwnerChanged`)。
//!    同じ戦争に両方が当てはまる場合は**消滅による決着を優先**する。
//! 3. 連鎖後の交戦関係を導出し、前の組との差(**敵対が解けた組**)を求める。別の戦争(従来の組を含む)で敵対が続く組は維持する。
//! 4. 敵対が解けた組どうしの進行中の戦闘を、勝敗・占領なしで中止する(割譲では M7-a の戦闘中止に合流させる)。
//! 5. 敵対が解けた組の占領(法的所有国と実効支配国が解けた組)を法的所有国へ戻す。
//! 6. 師団の帰還(下記)。帰還した師団の経路・目的地は消す。それ以外の移動命令は触らず、次の `advance_day` の経路検証に任せる。
//! 7. 休戦(明示した講和だけ。代表国間で [`TRUCE_DAYS`] 日)。消滅した国との休戦は消す。
//! 8. 補給の到達結果の無効化(実効支配が変わったプロヴィンスの前後の国、敵対が解けた組の国、移転の無効化の対象)。
//!
//! 師団の帰還の対象: 移転・正常化のあとの位置で、**自国の支配でも交戦相手の支配でもない**プロヴィンス(支配国あり)にいて、かつ(位置の支配国・位置の旧支配国・法的所有国のいずれかと)
//! **敵対が今回解けた**師団。帰還先は M7-a と同じ(隣接する自国支配のプロヴィンス → 自国支配の最小 `ProvinceId`、どちらも進行中の戦闘なし。なければ解散 `NoReturnProvince`)。
//!
//! 限界: 戦争点数・降伏・講和の受諾・停滞による自動講和はない。`Conflicts` と `Territory`・`MapMilitary` は別の値で、呼び出し側が 3 つを渡す。
//! 記録のある世界では、移転は終結 API 経由で行う(単独の `apply_transfer` は戦争記録を整理しないので、`Conflicts::check_consistency` が不一致を検出する)。

use super::battle::{CancelledBattle, MapBattleId, MapBattleStatus};
use super::ids::{MapStateId, ProvinceId};
use super::model::WorldModel;
use super::movement::{MapDivisionId, MapMilitary};
use super::territory::{ControlChange, DisbandReason, Disbanded, Relocation, Territory, TransferContext, TransferError, TransferOutcome, TransferRequest, relocate_division, return_destination};
use crate::common::CountryId;
use std::collections::{BTreeMap, BTreeSet};

/// 明示した講和の休戦日数(代表国間。旧 `execute_peace_settlement` と同じ 5 年)。
pub const TRUCE_DAYS: u32 = 1825;

/// 新マップ経路の戦争 ID(旧 `common::WarId` とは別の型。数値で対応付けない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapWarId(pub u32);

impl std::fmt::Display for MapWarId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MapWar#{}", self.0)
    }
}

/// 陣営(代表国 1 と、支持国 = 代表国以外の参加国)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarSide {
    pub primary: CountryId,
    /// 支持国(`CountryId` 昇順、代表国を含まない、重複なし)。
    pub supporters: Vec<CountryId>,
}

impl WarSide {
    pub fn new(primary: CountryId, supporters: impl IntoIterator<Item = CountryId>) -> WarSide {
        let mut s: Vec<CountryId> = supporters.into_iter().collect();
        s.sort_by_key(|c| c.0);
        s.dedup();
        WarSide { primary, supporters: s }
    }

    /// 参加国(代表国、続いて支持国)。
    pub fn members(&self) -> impl Iterator<Item = CountryId> + '_ {
        std::iter::once(self.primary).chain(self.supporters.iter().copied())
    }

    pub fn contains(&self, c: CountryId) -> bool {
        self.members().any(|m| m == c)
    }
}

/// 戦争の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarResult {
    WhitePeace,
    AttackerVictory,
    DefenderVictory,
}

/// 終結の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// 明示した白紙講和。
    WhitePeace,
    /// 明示した割譲(攻撃側の勝利)。
    Cession,
    /// 攻撃側の代表国が消滅した(防御側の勝利)。
    AttackerEliminated,
    /// 防御側の代表国が消滅した(攻撃側の勝利)。
    DefenderEliminated,
    /// 目標のステートの所有国が割譲で変わり、目標を達成できなくなった(白紙講和で失効)。
    TargetOwnerChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarEnd {
    pub day: u32,
    pub result: WarResult,
    pub reason: EndReason,
}

/// 戦争の記録。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarRecord {
    pub id: MapWarId,
    pub attacker: WarSide,
    pub defender: WarSide,
    /// 戦争目標の対象ステート(1 件)。
    pub target_state: MapStateId,
    pub start_day: u32,
    /// 終結していなければ `None`(稼働中)。
    pub end: Option<WarEnd>,
}

impl WarRecord {
    pub fn is_active(&self) -> bool {
        self.end.is_none()
    }
}

/// 記録の構造の不正。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictError {
    DuplicateWar(MapWarId),
    /// 代表国が両陣営で同じ。
    SameCountryBothSides { war: MapWarId, country: CountryId },
    /// 同じ国が両陣営に参加している。
    CountryOnBothSides { war: MapWarId, country: CountryId },
    /// 稼働中の戦争の組が、記録を持たない従来の組と重なっている。
    DuplicatesLegacyPair { war: MapWarId, a: CountryId, b: CountryId },
    /// 従来の組が自国との組。
    SelfWar(CountryId),
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictError::DuplicateWar(w) => write!(f, "{w} が重複しています"),
            ConflictError::SameCountryBothSides { war, country } => write!(f, "{war}: 両陣営の代表国が同じ CountryId {} です", country.0),
            ConflictError::CountryOnBothSides { war, country } => write!(f, "{war}: CountryId {} が両陣営に参加しています", country.0),
            ConflictError::DuplicatesLegacyPair { war, a, b } => write!(f, "{war}: 組 {}-{} が、記録のない交戦関係(wars)と重なっています", a.0, b.0),
            ConflictError::SelfWar(c) => write!(f, "CountryId {} の自国との交戦関係があります", c.0),
        }
    }
}

impl std::error::Error for ConflictError {}

fn key(a: CountryId, b: CountryId) -> (usize, usize) {
    (a.0.min(b.0), a.0.max(b.0))
}

fn pair(k: (usize, usize)) -> (CountryId, CountryId) {
    (CountryId(k.0), CountryId(k.1))
}

fn pairs_of(w: &WarRecord) -> impl Iterator<Item = (usize, usize)> + '_ {
    w.attacker.members().flat_map(move |a| w.defender.members().map(move |d| key(a, d)))
}

/// 戦争の記録・従来の交戦関係・休戦の集合。
#[derive(Debug, Clone, PartialEq)]
pub struct Conflicts {
    wars: BTreeMap<MapWarId, WarRecord>,
    legacy: BTreeSet<(usize, usize)>,
    /// 休戦の終了日(`(小さい CountryId, 大きい CountryId)` → 日。その日の**前日まで**休戦で、その日から切れる)。
    truces: BTreeMap<(usize, usize), u32>,
}

impl Conflicts {
    /// 記録と従来の組から作る(入力の順序に依存しない)。
    pub fn new(records: impl IntoIterator<Item = WarRecord>, legacy: impl IntoIterator<Item = (CountryId, CountryId)>) -> Result<Conflicts, ConflictError> {
        let mut wars = BTreeMap::new();
        for r in records {
            if r.attacker.primary == r.defender.primary {
                return Err(ConflictError::SameCountryBothSides { war: r.id, country: r.attacker.primary });
            }
            if let Some(c) = r.attacker.members().find(|c| r.defender.contains(*c)) {
                return Err(ConflictError::CountryOnBothSides { war: r.id, country: c });
            }
            let id = r.id;
            if wars.insert(id, r).is_some() {
                return Err(ConflictError::DuplicateWar(id));
            }
        }
        let mut set = BTreeSet::new();
        for (a, b) in legacy {
            if a == b {
                return Err(ConflictError::SelfWar(a));
            }
            set.insert(key(a, b));
        }
        let c = Conflicts { wars, legacy: set, truces: BTreeMap::new() };
        for w in c.wars.values().filter(|w| w.is_active()) {
            if let Some(k) = pairs_of(w).find(|k| c.legacy.contains(k)) {
                let (a, b) = pair(k);
                return Err(ConflictError::DuplicatesLegacyPair { war: w.id, a, b });
            }
        }
        Ok(c)
    }

    pub fn war(&self, id: MapWarId) -> Option<&WarRecord> {
        self.wars.get(&id)
    }

    /// 全記録(`MapWarId` 昇順。終結済みを含む)。
    pub fn wars(&self) -> impl Iterator<Item = &WarRecord> + '_ {
        self.wars.values()
    }

    pub fn active_wars(&self) -> impl Iterator<Item = &WarRecord> + '_ {
        self.wars.values().filter(|w| w.is_active())
    }

    /// 記録を持たない従来の組(昇順)。
    pub fn legacy_pairs(&self) -> Vec<(CountryId, CountryId)> {
        self.legacy.iter().map(|k| pair(*k)).collect()
    }

    fn derived_pairs(wars: &BTreeMap<MapWarId, WarRecord>, legacy: &BTreeSet<(usize, usize)>) -> BTreeSet<(usize, usize)> {
        let mut out: BTreeSet<(usize, usize)> = legacy.clone();
        for w in wars.values().filter(|w| w.is_active()) {
            out.extend(pairs_of(w));
        }
        out
    }

    /// 現在の敵対する組(稼働中の戦争の両陣営の全組み合わせ + 従来の組。昇順)。
    pub fn hostile_pairs(&self) -> Vec<(CountryId, CountryId)> {
        Self::derived_pairs(&self.wars, &self.legacy).into_iter().map(pair).collect()
    }

    /// 2 国が敵対しているか(双方向)。
    pub fn at_war(&self, a: CountryId, b: CountryId) -> bool {
        a != b && Self::derived_pairs(&self.wars, &self.legacy).contains(&key(a, b))
    }

    /// 交戦関係を `MapMilitary` へ反映する(導出の結果で置き換える。何度呼んでも同じ)。
    pub fn sync(&self, military: &mut MapMilitary) {
        military.wars = Self::derived_pairs(&self.wars, &self.legacy).into_iter().collect();
    }

    /// `MapMilitary` の交戦関係が、記録から導出した組と一致するか。
    pub fn check_consistency(&self, military: &MapMilitary) -> Result<(), String> {
        let derived: Vec<(usize, usize)> = Self::derived_pairs(&self.wars, &self.legacy).into_iter().collect();
        if military.wars == derived {
            Ok(())
        } else {
            Err(format!("交戦関係 {:?} が、戦争の記録から導出した {:?} と違います", military.wars, derived))
        }
    }

    /// 2 国の休戦の終了日(休戦がなければ `None`。期限が切れていても記録は残るので、判定は [`Conflicts::in_truce`])。
    pub fn truce_until(&self, a: CountryId, b: CountryId) -> Option<u32> {
        self.truces.get(&key(a, b)).copied()
    }

    /// `day` に 2 国が休戦中か(終了日の前日まで true、終了日から false。旧 `get_truce_until` の `is_at_least` と同じ境界)。
    pub fn in_truce(&self, a: CountryId, b: CountryId, day: u32) -> bool {
        self.truce_until(a, b).is_some_and(|until| day < until)
    }

    /// 休戦の一覧(昇順)。
    pub fn truces(&self) -> impl Iterator<Item = (CountryId, CountryId, u32)> + '_ {
        self.truces.iter().map(|(k, d)| (CountryId(k.0), CountryId(k.1), *d))
    }

    /// 世界との照合(ステートが世界にあり、参加国が存続していること)。
    pub fn check_world(&self, model: &WorldModel, territory: &Territory) -> Result<(), String> {
        for w in self.wars.values().filter(|w| w.is_active()) {
            if model.state(w.target_state).is_none() {
                return Err(format!("{}: 目標 {} は世界にありません", w.id, w.target_state));
            }
            if let Some(c) = w.attacker.members().chain(w.defender.members()).find(|c| !territory.is_active(*c)) {
                return Err(format!("{}: 参加国 CountryId {} は存続していません", w.id, c.0));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 要求・拒否・結果

/// 講和の条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeaceTerms {
    /// 白紙講和(領土は動かない。休戦あり)。
    White,
    /// 目標ステートの割譲(防御側の代表国 → 攻撃側の代表国。攻撃側の勝利)。`cancel_battles` は M7-a と同じ**明示**の戦闘中止
    /// (移転ステート内の、敵対が続く組の戦闘を中止する場合に指定する。敵対が解ける組の戦闘は自動で中止する)。
    Cession { cancel_battles: Vec<MapBattleId> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndWarRequest {
    pub war: MapWarId,
    pub terms: PeaceTerms,
    /// 終結日(日番号。休戦の起点。開始日より前は不可)。
    pub day: u32,
}

impl EndWarRequest {
    pub fn white_peace(war: MapWarId, day: u32) -> EndWarRequest {
        EndWarRequest { war, terms: PeaceTerms::White, day }
    }

    pub fn cession(war: MapWarId, day: u32) -> EndWarRequest {
        EndWarRequest { war, terms: PeaceTerms::Cession { cancel_battles: Vec::new() }, day }
    }

    pub fn with_cancel_battles(mut self, ids: impl IntoIterator<Item = MapBattleId>) -> EndWarRequest {
        if let PeaceTerms::Cession { cancel_battles } = &mut self.terms {
            *cancel_battles = ids.into_iter().collect();
        }
        self
    }

    fn normalized(&self) -> EndWarRequest {
        let mut r = self.clone();
        if let PeaceTerms::Cession { cancel_battles } = &mut r.terms {
            cancel_battles.sort();
            cancel_battles.dedup();
        }
        r
    }
}

/// 要求の拒否(何も変えない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndWarError {
    UnknownWar(MapWarId),
    /// 終結済み(終結の結果と日を持つ)。割譲・休戦は二重に適用されない。
    AlreadyEnded { war: MapWarId, end: WarEnd },
    /// 終結日が開始日より前。
    DayBeforeStart { war: MapWarId, start_day: u32, day: u32 },
    /// `MapMilitary` の交戦関係が戦争の記録から導出した組と違う(記録を経由しない変更があった)。
    HostilityOutOfSync,
    /// 割譲の移転(M7-a の検証)が拒否した。
    Transfer(TransferError),
}

impl std::fmt::Display for EndWarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EndWarError::UnknownWar(w) => write!(f, "{w} は存在しません"),
            EndWarError::AlreadyEnded { war, end } => write!(f, "{war} は {} 日に終結済みです(割譲・休戦は二重に適用しません)", end.day),
            EndWarError::DayBeforeStart { war, start_day, day } => write!(f, "{war} の開始日 {start_day} より前の日 {day} には終結できません"),
            EndWarError::HostilityOutOfSync => write!(f, "交戦関係が戦争の記録と一致しません(記録を経由しない変更があります)"),
            EndWarError::Transfer(e) => write!(f, "割譲を適用できません: {e}"),
        }
    }
}

impl std::error::Error for EndWarError {}

/// 古い計画の理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndWarStale {
    /// 現在の状態では要求自体が成り立たない。
    Invalid(EndWarError),
    /// 要求は成り立つが、結果が計画と違う。
    OutcomeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndWarApplyError {
    /// この戦争は既に終結している(計画の適用済みを含む)。
    AlreadyEnded { war: MapWarId, end: WarEnd },
    Stale(EndWarStale),
}

impl std::fmt::Display for EndWarApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EndWarApplyError::AlreadyEnded { war, end } => write!(f, "{war} は {} 日に終結済みです(二重に適用しません)", end.day),
            EndWarApplyError::Stale(EndWarStale::Invalid(e)) => write!(f, "現在の状態では成り立たない古い計画です: {e}"),
            EndWarApplyError::Stale(EndWarStale::OutcomeChanged) => write!(f, "現在の状態では結果が計画と違うため、古い計画です"),
        }
    }
}

impl std::error::Error for EndWarApplyError {}

/// 終結した戦争。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedWar {
    pub war: MapWarId,
    pub result: WarResult,
    pub reason: EndReason,
}

/// 陣営から除いた支持国(消滅した国)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupporterRemoved {
    pub war: MapWarId,
    pub country: CountryId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruceSet {
    pub a: CountryId,
    pub b: CountryId,
    /// 終了日(この日から休戦は切れる)。
    pub until: u32,
}

/// 終結の結果(計画にも適用にも同じ形で出る)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndWarOutcome {
    pub day: u32,
    /// 終結した戦争(要求の戦争、消滅による決着(ID 昇順)、目標の所有国が変わった失効(ID 昇順))。
    pub ended: Vec<EndedWar>,
    pub supporters_removed: Vec<SupporterRemoved>,
    /// 割譲の移転の結果(白紙講和は `None`)。
    pub transfer: Option<TransferOutcome>,
    /// 敵対が解けた組(昇順)。
    pub hostility_ended: Vec<(CountryId, CountryId)>,
    /// 終結した戦争の組のうち、別の戦争(従来の組を含む)で敵対が続くため維持した組(昇順)。
    pub hostility_kept: Vec<(CountryId, CountryId)>,
    /// 敵対が解けた組の戦闘のうち、割譲の移転の外で中止したもの(割譲では移転の結果 `transfer.cancelled_battles` に入る)。
    pub cancelled_battles: Vec<CancelledBattle>,
    /// 占領を法的所有国へ戻した支配の変更(プロヴィンス昇順)。
    pub occupation_normalized: Vec<ControlChange>,
    /// 帰還した師団(ID 昇順)。
    pub returned: Vec<Relocation>,
    /// 帰還先がなく解散した師団(ID 昇順。戦闘による撃破とは別)。
    pub disbanded: Vec<Disbanded>,
    pub truce: Option<TruceSet>,
    /// 消滅した国との休戦で消したもの(昇順)。
    pub truces_cleared: Vec<(CountryId, CountryId)>,
    /// 補給の到達結果を無効化した国家(昇順、重複なし)。
    pub invalidated_countries: Vec<CountryId>,
}

impl EndWarOutcome {
    /// 中止した戦闘すべて(移転の中のものと外のもの。`MapBattleId` 昇順)。
    pub fn all_cancelled_battles(&self) -> Vec<&CancelledBattle> {
        let mut v: Vec<&CancelledBattle> = self.cancelled_battles.iter().chain(self.transfer.iter().flat_map(|t| t.cancelled_battles.iter())).collect();
        v.sort_by_key(|b| b.battle);
        v
    }
}

/// 計画(状態を変えずに作った結果)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndWarPlan {
    request: EndWarRequest,
    outcome: EndWarOutcome,
}

impl EndWarPlan {
    pub fn request(&self) -> &EndWarRequest {
        &self.request
    }
    pub fn outcome(&self) -> &EndWarOutcome {
        &self.outcome
    }
}

// ---------------------------------------------------------------- 計画・適用

/// 事前検証と計画の作成。引数はどれも変更しない。
pub fn plan_end_war(ctx: TransferContext<'_>, territory: &Territory, conflicts: &Conflicts, military: &MapMilitary, request: &EndWarRequest) -> Result<EndWarPlan, EndWarError> {
    let request = request.normalized();
    let mut t = territory.clone();
    let mut c = conflicts.clone();
    let mut m = military.clone();
    let outcome = execute(ctx, &mut t, &mut c, &mut m, &request)?;
    Ok(EndWarPlan { request, outcome })
}

/// 計画を適用する。戦争が終結済みなら二重に適用せず拒否する。適用の直前に現在の状態で再検証し、計画と一致しなければ何も変えずに拒否する。
pub fn apply_end_war(
    ctx: TransferContext<'_>,
    territory: &mut Territory,
    conflicts: &mut Conflicts,
    military: &mut MapMilitary,
    plan: &EndWarPlan,
) -> Result<EndWarOutcome, EndWarApplyError> {
    if let Some(w) = conflicts.war(plan.request.war)
        && let Some(end) = w.end
    {
        return Err(EndWarApplyError::AlreadyEnded { war: w.id, end });
    }
    {
        let mut t = territory.clone();
        let mut c = conflicts.clone();
        let mut m = military.clone();
        match execute(ctx, &mut t, &mut c, &mut m, &plan.request) {
            Err(e) => return Err(EndWarApplyError::Stale(EndWarStale::Invalid(e))),
            Ok(o) if o != plan.outcome => return Err(EndWarApplyError::Stale(EndWarStale::OutcomeChanged)),
            Ok(_) => {}
        }
    }
    execute(ctx, territory, conflicts, military, &plan.request).map_err(|e| EndWarApplyError::Stale(EndWarStale::Invalid(e)))
}

/// 計画と適用をまとめて行う。終結済みの戦争は `AlreadyEnded`。
pub fn end_war(
    ctx: TransferContext<'_>,
    territory: &mut Territory,
    conflicts: &mut Conflicts,
    military: &mut MapMilitary,
    request: &EndWarRequest,
) -> Result<EndWarOutcome, EndWarApplyError> {
    let plan = plan_end_war(ctx, territory, conflicts, military, request).map_err(|e| match e {
        EndWarError::AlreadyEnded { war, end } => EndWarApplyError::AlreadyEnded { war, end },
        e => EndWarApplyError::Stale(EndWarStale::Invalid(e)),
    })?;
    apply_end_war(ctx, territory, conflicts, military, &plan)
}

/// 終結の連鎖(作業用の記録に対して決める。状態は変えない)。
struct Cascade {
    wars: BTreeMap<MapWarId, WarRecord>,
    ended: Vec<EndedWar>,
    supporters_removed: Vec<SupporterRemoved>,
}

fn cascade(conflicts: &Conflicts, req: &EndWarRequest, eliminated: Option<CountryId>, cession_state: Option<MapStateId>) -> Cascade {
    let mut wars = conflicts.wars.clone();
    let mut ended: Vec<EndedWar> = Vec::new();
    let mut supporters_removed: Vec<SupporterRemoved> = Vec::new();
    let (result, reason) = match req.terms {
        PeaceTerms::White => (WarResult::WhitePeace, EndReason::WhitePeace),
        PeaceTerms::Cession { .. } => (WarResult::AttackerVictory, EndReason::Cession),
    };
    let mut ended_ids: BTreeSet<MapWarId> = BTreeSet::new();
    ended_ids.insert(req.war);
    ended.push(EndedWar { war: req.war, result, reason });
    let active_ids = |wars: &BTreeMap<MapWarId, WarRecord>, ended_ids: &BTreeSet<MapWarId>| -> Vec<MapWarId> { wars.values().filter(|w| w.is_active() && !ended_ids.contains(&w.id)).map(|w| w.id).collect() };
    // 1) 消滅による決着(優先): 代表国なら対陣営の勝利、支持国としてのみなら陣営から除く
    if let Some(e) = eliminated {
        for id in active_ids(&wars, &ended_ids) {
            let w = wars.get_mut(&id).expect("exists");
            if w.attacker.primary == e {
                ended_ids.insert(id);
                ended.push(EndedWar { war: id, result: WarResult::DefenderVictory, reason: EndReason::AttackerEliminated });
            } else if w.defender.primary == e {
                ended_ids.insert(id);
                ended.push(EndedWar { war: id, result: WarResult::AttackerVictory, reason: EndReason::DefenderEliminated });
            } else {
                let mut removed = false;
                for side in [&mut w.attacker, &mut w.defender] {
                    if side.supporters.contains(&e) {
                        side.supporters.retain(|c| *c != e);
                        removed = true;
                    }
                }
                if removed {
                    supporters_removed.push(SupporterRemoved { war: id, country: e });
                }
            }
        }
    }
    // 2) 割譲したステートを目標とする残りの稼働中の戦争は白紙講和で失効(消滅による決着が優先なので、ここは残りだけ)
    if let Some(state) = cession_state {
        for id in active_ids(&wars, &ended_ids) {
            if wars[&id].target_state == state {
                ended_ids.insert(id);
                ended.push(EndedWar { war: id, result: WarResult::WhitePeace, reason: EndReason::TargetOwnerChanged });
            }
        }
    }
    // 終結を記録に反映(終了日は呼び出し側が入れる)
    for e in &ended {
        wars.get_mut(&e.war).expect("exists").end = Some(WarEnd { day: req.day, result: e.result, reason: e.reason });
    }
    Cascade { wars, ended, supporters_removed }
}

fn execute(ctx: TransferContext<'_>, territory: &mut Territory, conflicts: &mut Conflicts, military: &mut MapMilitary, req: &EndWarRequest) -> Result<EndWarOutcome, EndWarError> {
    let TransferContext { model, graph } = ctx;
    // ---- 検証(ここまで状態を変えない)
    let war = conflicts.war(req.war).ok_or(EndWarError::UnknownWar(req.war))?;
    if let Some(end) = war.end {
        return Err(EndWarError::AlreadyEnded { war: war.id, end });
    }
    if req.day < war.start_day {
        return Err(EndWarError::DayBeforeStart { war: war.id, start_day: war.start_day, day: req.day });
    }
    conflicts.check_consistency(military).map_err(|_| EndWarError::HostilityOutOfSync)?;
    let (attacker, defender, target) = (war.attacker.primary, war.defender.primary, war.target_state);
    let cession = matches!(req.terms, PeaceTerms::Cession { .. });
    // 割譲で消滅する国(防御側の代表国が目標だけを所有している)
    let eliminated = (cession && territory.owned_states(defender) == vec![target]).then_some(defender);

    let before = Conflicts::derived_pairs(&conflicts.wars, &conflicts.legacy);
    let cas = cascade(conflicts, req, eliminated, cession.then_some(target));
    let mut legacy_after = conflicts.legacy.clone();
    if let Some(e) = eliminated {
        legacy_after.retain(|k| k.0 != e.0 && k.1 != e.0);
    }
    let after = Conflicts::derived_pairs(&cas.wars, &legacy_after);
    let hostility_ended: BTreeSet<(usize, usize)> = before.difference(&after).copied().collect();
    let ended_war_pairs: BTreeSet<(usize, usize)> = cas.ended.iter().flat_map(|e| pairs_of(&conflicts.wars[&e.war])).collect();
    let hostility_kept: Vec<(CountryId, CountryId)> = ended_war_pairs.intersection(&after).copied().map(pair).collect();
    // 敵対が解けた組の進行中の戦闘(消滅国の戦闘は移転が自動で中止するが、組の差にも入る)
    let dissolved: BTreeSet<MapBattleId> = military
        .battles()
        .filter(|b| b.status == MapBattleStatus::Ongoing && hostility_ended.contains(&key(b.attacker_country, b.defender_country)))
        .map(|b| b.id)
        .collect();

    // 割譲: M7-a の移転(検証が失敗すれば何も変えずに返る)
    let mut transfer: Option<TransferOutcome> = None;
    if let PeaceTerms::Cession { cancel_battles } = &req.terms {
        let mut cancel: BTreeSet<MapBattleId> = cancel_battles.iter().copied().collect();
        cancel.extend(dissolved.iter().copied());
        let treq = TransferRequest::new(target, defender, attacker).with_cancel_battles(cancel);
        transfer = Some(super::territory::execute(ctx, territory, military, &treq).map_err(EndWarError::Transfer)?);
    }

    // ---- 変更(ここから先は失敗しない)
    let mut invalidate: BTreeSet<usize> = BTreeSet::new();
    let cancelled_battles: Vec<CancelledBattle> = if cession {
        Vec::new()
    } else {
        let ids: BTreeSet<MapBattleId> = dissolved.clone();
        military.cancel_battles_unchecked(&ids)
    };
    // 記録・従来の組・交戦関係
    conflicts.wars = cas.wars;
    conflicts.legacy = legacy_after;
    conflicts.sync(military);
    // 占領の正常化: 敵対が解けた組の(法的所有国, 支配国)は、所有国の支配へ戻す
    let mut occupation_normalized: Vec<ControlChange> = Vec::new();
    let mut held: Vec<(ProvinceId, CountryId)> = military.control().iter().collect();
    held.sort_by_key(|(p, _)| *p);
    let mut pre_control: BTreeMap<ProvinceId, Option<CountryId>> = BTreeMap::new();
    for d in military.divisions() {
        pre_control.insert(d.province, military.control().controller(d.province));
    }
    for (p, x) in held {
        let Some(owner) = model.state_of(p).and_then(|s| territory.owner(s)) else { continue };
        if owner != x && hostility_ended.contains(&key(owner, x)) {
            military.control.set(p, Some(owner));
            occupation_normalized.push(ControlChange { province: p, previous: Some(x), new: Some(owner) });
            invalidate.insert(x.0);
            invalidate.insert(owner.0);
        }
    }
    // 師団の帰還(ID 昇順)
    let mut candidates: Vec<MapDivisionId> = Vec::new();
    for d in military.divisions() {
        let here = military.control().controller(d.province);
        let Some(x) = here else { continue };
        if x == d.owner || military.at_war(d.owner, x) {
            continue;
        }
        let owner_here = model.state_of(d.province).and_then(|s| territory.owner(s));
        let touched = [Some(x), pre_control.get(&d.province).copied().flatten(), owner_here].into_iter().flatten().any(|y| y != d.owner && hostility_ended.contains(&key(d.owner, y)));
        if touched {
            candidates.push(d.id);
        }
    }
    let mut returned: Vec<Relocation> = Vec::new();
    let mut disbanded: Vec<Disbanded> = Vec::new();
    for id in candidates {
        let (owner, from) = {
            let d = military.division(id).expect("exists");
            (d.owner, d.province)
        };
        match return_destination(graph, military, owner, from) {
            Some(to) => {
                relocate_division(military, id, to);
                returned.push(Relocation { division: id, owner, from, to });
            }
            None => {
                military.divisions.remove(&id);
                disbanded.push(Disbanded { division: id, owner, at: from, reason: DisbandReason::NoReturnProvince });
            }
        }
    }
    // 休戦(明示した講和だけ。代表国間)。消滅した国との休戦は消す
    let until = req.day.saturating_add(TRUCE_DAYS);
    conflicts.truces.insert(key(attacker, defender), until);
    let mut truce = Some(TruceSet { a: CountryId(attacker.0.min(defender.0)), b: CountryId(attacker.0.max(defender.0)), until });
    let mut truces_cleared: Vec<(CountryId, CountryId)> = Vec::new();
    if let Some(e) = eliminated {
        let doomed: Vec<(usize, usize)> = conflicts.truces.keys().filter(|k| k.0 == e.0 || k.1 == e.0).copied().collect();
        for k in doomed {
            conflicts.truces.remove(&k);
            truces_cleared.push(pair(k));
        }
        if truce.as_ref().is_some_and(|t| t.a == e || t.b == e) {
            truce = None;
        }
    }
    // 補給の無効化
    for k in &hostility_ended {
        invalidate.insert(k.0);
        invalidate.insert(k.1);
    }
    if let Some(t) = &transfer {
        invalidate.extend(t.invalidated_countries.iter().map(|c| c.0));
    }
    let invalidated_countries: Vec<CountryId> = invalidate.into_iter().map(CountryId).collect();
    military.invalidate_reach(&invalidated_countries);

    Ok(EndWarOutcome {
        day: req.day,
        ended: cas.ended,
        supporters_removed: cas.supporters_removed,
        transfer,
        hostility_ended: hostility_ended.into_iter().map(pair).collect(),
        hostility_kept,
        cancelled_battles,
        occupation_normalized,
        returned,
        disbanded,
        truce,
        truces_cleared,
        invalidated_countries,
    })
}
