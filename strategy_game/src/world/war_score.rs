//! P32-043 (M7-c): 戦争点数と降伏判定(純粋な Rust。Bevy 非依存)。**判定結果を返すところまで**で、結果を使った講和の適用は M7-d。
//!
//! 入力はすべて読み取り専用: 戦争記録(`Conflicts`)・法的所有(`Territory`)・実効支配と師団(`MapMilitary`)・戦闘の集計(`BattleLedger`)。
//! 旧経路(`war::war_score`・`war::capitulation`)の規則を基にし、**占領率だけ S 案**に置き換えた。
//!
//! ## 占領率(S 案)
//! 陣営 X(代表国 pX)が相手の代表国 pO に対して持つ占領率 = 「pO が**法的に所有する**ステートそれぞれの `pX が支配するプロヴィンス数 ÷ 所属総数`」の**単純平均**。
//! - 地図のプロヴィンスは陸だけなので、世界のステートはすべて陸ステート。所属プロヴィンスが 0 件のステートは**平均から除く**(分母 0 を作らない)。
//! - 第三国・支持国・支配国なしのプロヴィンスは分母に含み、分子には入らない(支持国の支配は pX の占領に数えない。代表国の支配だけ)。
//! - **丸めない**: 平均は既約分数(`u128` の厳密な有理数)で持つ。60% 以上の判定は `分子×10 ≥ 分母×6` の整数比較で、ステートの順序・途中の丸めに依存しない。
//!   占領点は `⌊占領率 × 40⌋` を整数除算で一度だけ切り捨てる。分母が `u128` を超える病的な入力は [`ScoreError::RateOverflow`](黙って丸めない)。
//! - 相手の代表国が所有するステートが 0 件(または全て所属 0 件)のとき、占領率は**定義しない**(`None`): 占領点は 0、占領率による降伏条件は**成立しない**。
//!   旧は「陸ステート 0 件なら降伏」としたが、新経路では国の消滅(M7-a/b)が別に扱うので、点数の入力にしない(意図した差)。
//!
//! ## 点数(攻撃側視点)
//! `総点 = clamp(目標 + 攻撃側占領点 − 防御側占領点 + 戦闘点, ±100)`。目標 = 目標ステート全域(所属 1 件以上で全件)を攻撃側代表国が支配なら +40。
//! 攻撃側占領点 = ⌊攻撃側の占領率 × 40⌋、防御側占領点 = ⌊防御側の占領率 × 40⌋(正の値で保持し、引く)。戦闘点 = clamp(勝利数の差 × 5, ±20)。
//!
//! ## 戦闘の集計([`BattleLedger`])
//! `MoveEvent::BattleFinished`(勝敗が確定した戦闘)だけを、`MapBattleId` ごとに**一度だけ**数える。中止(`cancel_battles`)は終了イベントを出さず、守備隊のいない占領は戦闘ではないので数えない。
//! 旧と同じく、組織の脱落・両者消滅・損失 0 による防御側勝利も確定した結果として数える。交戦国の組(攻撃国・防御国)から**稼働中の戦争が 1 件に決まるときだけ**計上し、
//! 0 件(従来の組の戦争など)・複数件は計上せず、理由を残す(`skipped`)。同じ戦闘は二度と再計上しない(スキップした戦闘も)。
//!
//! ## 降伏([`assess_war`])
//! - 防御側: 目標全域を攻撃側代表国が支配 **かつ**(攻撃側の占領率 ≥ 60% **または** 防御側代表国に戦闘可能な師団がない)。
//! - 攻撃側: (防御側の占領率 ≥ 60%)**または**(攻撃側代表国に戦闘可能な師団がなく、防御側代表国が攻撃側代表国の所有ステートを 1 件以上**全域**支配)。
//! - 戦闘可能 = 人員が 0 でない師団(新経路は撃破された師団を除去するので、旧の「撃破されていない」と同じ)。
//! - 双方が成立: 総点 > 0 なら防御側、それ以外(0 を含む)は攻撃側が降伏。終結済みの戦争は判定しない([`ScoreError::AlreadyEnded`]。[`assess_active_wars`] は除く)。
//!
//! 日次処理の外で呼ぶ読み取り専用の関数で、毎日の自動判定・講和の適用はしない(M7-d)。

use super::battle::MapBattleId;
use super::ids::MapStateId;
use super::model::WorldModel;
use super::movement::{BattleOutcome, MapMilitary, MoveEvent};
use super::state_control::{ControlSummaryError, StateControlKind, StateControlSummary, summarize_control};
use super::territory::Territory;
use super::war_record::{Conflicts, MapWarId, WarRecord};
use crate::common::CountryId;
use std::collections::{BTreeMap, BTreeSet};

/// 目標ステート全域の支配点。
pub const TARGET_POINTS: i32 = 40;
/// 占領率 100% のときの占領点。
pub const OCCUPATION_MAX_POINTS: u128 = 40;
/// 戦闘 1 勝の差あたりの点。
pub const BATTLE_POINTS_PER_WIN: i32 = 5;
/// 戦闘点の上限(±)。
pub const BATTLE_POINTS_CAP: i32 = 20;
/// 総点の上限(±)。
pub const TOTAL_POINTS_CAP: i32 = 100;
/// 占領率の降伏しきい値(60% = 6/10。以上で成立)。
pub const SURRENDER_RATE_NUM: u128 = 6;
pub const SURRENDER_RATE_DEN: u128 = 10;

/// 厳密な有理数(既約。分母 > 0)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub num: u128,
    pub den: u128,
}

fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl Ratio {
    fn new(num: u128, den: u128) -> Ratio {
        let g = gcd(num, den).max(1);
        Ratio { num: num / g, den: den / g }
    }

    fn checked_add(self, other: Ratio) -> Option<Ratio> {
        let g = gcd(self.den, other.den);
        let l = (self.den / g).checked_mul(other.den)?;
        let a = self.num.checked_mul(l / self.den)?;
        let b = other.num.checked_mul(l / other.den)?;
        Some(Ratio::new(a.checked_add(b)?, l))
    }

    /// `⌊self × scale⌋`(整数除算。オーバーフローは `None`)。
    pub fn floor_times(&self, scale: u128) -> Option<u128> {
        Some(self.num.checked_mul(scale)? / self.den)
    }

    /// `self ≥ num/den`(整数比較)。
    pub fn at_least(&self, num: u128, den: u128) -> Option<bool> {
        Some(self.num.checked_mul(den)? >= num.checked_mul(self.den)?)
    }

    /// 表示用の小数(判定には使わない)。
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// 1 ステートぶんの占領の内訳。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateOccupation {
    pub state: MapStateId,
    /// 占領側の代表国が支配するプロヴィンス数。
    pub controlled: usize,
    /// 所属総数(1 以上)。
    pub total: usize,
}

/// 一方の陣営の占領率(占領側 → 相手の代表国の所有ステート)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupationRate {
    pub occupier: CountryId,
    pub owner: CountryId,
    /// 対象のステート(`MapStateId` 昇順。所属 0 件は含まない)。
    pub states: Vec<StateOccupation>,
    /// 平均(対象が 0 件なら `None`)。
    pub rate: Option<Ratio>,
}

impl OccupationRate {
    /// 占領点 `⌊占領率 × 40⌋`(定義しなければ 0)。
    pub fn points(&self) -> Result<i32, ScoreError> {
        match self.rate {
            None => Ok(0),
            Some(r) => r.floor_times(OCCUPATION_MAX_POINTS).map(|p| p as i32).ok_or(ScoreError::RateOverflow),
        }
    }

    /// 占領率が 60% 以上か(定義しなければ `false`)。
    pub fn reaches_surrender_rate(&self) -> Result<bool, ScoreError> {
        match self.rate {
            None => Ok(false),
            Some(r) => r.at_least(SURRENDER_RATE_NUM, SURRENDER_RATE_DEN).ok_or(ScoreError::RateOverflow),
        }
    }
}

/// 点数・降伏判定を作れない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreError {
    /// 終結済みの戦争は判定しない。
    AlreadyEnded(MapWarId),
    /// 目標ステートが世界にない。
    UnknownTarget { war: MapWarId, state: MapStateId },
    /// 支配の集計に失敗した(所属表の不整合)。
    Control(ControlSummaryError),
    /// 厳密な有理数が `u128` に収まらない(丸めずに拒否する)。
    RateOverflow,
}

impl std::fmt::Display for ScoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScoreError::AlreadyEnded(w) => write!(f, "{w} は終結済みです(判定しません)"),
            ScoreError::UnknownTarget { war, state } => write!(f, "{war} の目標 {state} は世界にありません"),
            ScoreError::Control(e) => write!(f, "支配の集計に失敗しました: {e}"),
            ScoreError::RateOverflow => write!(f, "占領率の分数が u128 に収まりません(丸めずに拒否します)"),
        }
    }
}

impl std::error::Error for ScoreError {}

/// `occupier` が `owner` の所有ステートに対して持つ占領率(S 案)。
pub fn occupation_rate(summary: &StateControlSummary, occupier: CountryId, owner: CountryId) -> Result<OccupationRate, ScoreError> {
    let mut states = Vec::new();
    let mut sum: Option<Ratio> = None;
    for s in summary.states() {
        if s.legal_owner != Some(owner) || s.total == 0 {
            continue;
        }
        let controlled = s.count_of(occupier);
        states.push(StateOccupation { state: s.state, controlled, total: s.total });
        let term = Ratio::new(controlled as u128, s.total as u128);
        sum = Some(match sum {
            None => term,
            Some(acc) => acc.checked_add(term).ok_or(ScoreError::RateOverflow)?,
        });
    }
    let rate = match sum {
        None => None,
        Some(r) => Some(Ratio::new(r.num, r.den.checked_mul(states.len() as u128).ok_or(ScoreError::RateOverflow)?)),
    };
    Ok(OccupationRate { occupier, owner, states, rate })
}

// ---- 戦闘の集計 ----

/// 戦闘の計上結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerEntry {
    /// 戦争 `war` の `side_won` 側の勝利として計上した。
    Counted { battle: MapBattleId, war: MapWarId, attacker_side_won: bool },
    /// 計上しなかった(理由つき)。
    Skipped { battle: MapBattleId, reason: SkipReason },
    /// 既に処理済みの戦闘(二重計上しない)。
    Duplicate { battle: MapBattleId },
}

/// 計上しない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// 交戦国の組を両陣営に持つ稼働中の戦争がない(従来の組だけの敵対など)。
    NoWar,
    /// 交戦国の組を両陣営に持つ稼働中の戦争が複数あり、一意に決められない(候補は `MapWarId` 昇順)。
    Ambiguous(Vec<MapWarId>),
}

/// 戦闘の勝敗の集計(戦争ごと)。実行時状態で、保存は後続。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattleLedger {
    seen: BTreeSet<MapBattleId>,
    wins: BTreeMap<MapWarId, (u32, u32)>,
    skipped: Vec<(MapBattleId, SkipReason)>,
}

impl BattleLedger {
    pub fn new() -> BattleLedger {
        BattleLedger::default()
    }

    /// 戦争ごとの (攻撃側の勝利数, 防御側の勝利数)。
    pub fn wins(&self, war: MapWarId) -> (u32, u32) {
        self.wins.get(&war).copied().unwrap_or((0, 0))
    }

    /// 計上しなかった戦闘(記録順)。
    pub fn skipped(&self) -> &[(MapBattleId, SkipReason)] {
        &self.skipped
    }

    /// 1 件の確定した戦闘を計上する。`conflicts` は**その時点の**記録(稼働中の戦争だけが候補)。
    pub fn record(&mut self, conflicts: &Conflicts, battle: MapBattleId, attacker: CountryId, defender: CountryId, outcome: BattleOutcome) -> LedgerEntry {
        if !self.seen.insert(battle) {
            return LedgerEntry::Duplicate { battle };
        }
        let candidates: Vec<&WarRecord> = conflicts
            .active_wars()
            .filter(|w| (w.attacker.contains(attacker) && w.defender.contains(defender)) || (w.attacker.contains(defender) && w.defender.contains(attacker)))
            .collect();
        match candidates.as_slice() {
            [] => {
                self.skipped.push((battle, SkipReason::NoWar));
                LedgerEntry::Skipped { battle, reason: SkipReason::NoWar }
            }
            [w] => {
                let winner = match outcome {
                    BattleOutcome::AttackerWon => attacker,
                    BattleOutcome::DefenderWon => defender,
                };
                let attacker_side_won = w.attacker.contains(winner);
                let e = self.wins.entry(w.id).or_default();
                if attacker_side_won {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
                LedgerEntry::Counted { battle, war: w.id, attacker_side_won }
            }
            many => {
                let ids: Vec<MapWarId> = many.iter().map(|w| w.id).collect();
                let reason = SkipReason::Ambiguous(ids);
                self.skipped.push((battle, reason.clone()));
                LedgerEntry::Skipped { battle, reason }
            }
        }
    }

    /// 1 日ぶんのイベントから `BattleFinished` だけを `MapBattleId` 昇順で計上する(他のイベント・中止・守備隊なしの占領は数えない)。
    pub fn record_events(&mut self, conflicts: &Conflicts, events: &[MoveEvent]) -> Vec<LedgerEntry> {
        let mut finished: Vec<(MapBattleId, CountryId, CountryId, BattleOutcome)> = events
            .iter()
            .filter_map(|e| match e {
                MoveEvent::BattleFinished { battle, attacker, defender, outcome, .. } => Some((*battle, *attacker, *defender, *outcome)),
                _ => None,
            })
            .collect();
        finished.sort_by_key(|f| f.0);
        finished.into_iter().map(|(b, a, d, o)| self.record(conflicts, b, a, d, o)).collect()
    }
}

// ---- 点数 ----

/// 点数の内訳(攻撃側視点)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarScoreBreakdown {
    pub war: MapWarId,
    /// 目標ステート全域を攻撃側代表国が支配しているか。
    pub target_held: bool,
    pub war_goal_score: i32,
    /// 攻撃側が防御側の所有ステートに対して持つ占領率。
    pub attacker_occupation: OccupationRate,
    /// 防御側が攻撃側の所有ステートに対して持つ占領率。
    pub defender_occupation: OccupationRate,
    pub attacker_occupation_score: i32,
    /// 防御側の占領点(**正の値**。総点では引く)。
    pub defender_occupation_score: i32,
    pub attacker_battle_wins: u32,
    pub defender_battle_wins: u32,
    /// 上限(±20)適用後。
    pub battle_score: i32,
    /// 上限適用前の合計。
    pub raw_total: i32,
    /// 上限(±100)適用後。
    pub total_score: i32,
}

/// 降伏の理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefenderReason {
    /// 目標全域の支配に加え、攻撃側の占領率が 60% 以上。
    TargetHeldAndOccupation,
    /// 目標全域の支配に加え、防御側代表国に戦闘可能な師団がない。
    TargetHeldAndNoCombatReadyDivisions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackerReason {
    /// 防御側の占領率(攻撃側の自領の被占領率)が 60% 以上。
    OccupationRate,
    /// 戦闘可能な師団がなく、自領の陸ステートを 1 件以上(列挙)全域支配された。
    NoCombatReadyDivisionsAndStateHeld(Vec<MapStateId>),
}

/// 降伏の判定結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// どちらも降伏しない。
    None,
    DefenderSurrenders,
    AttackerSurrenders,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarAssessment {
    pub breakdown: WarScoreBreakdown,
    /// 防御側の成立した条件(空 = 不成立)。
    pub defender_reasons: Vec<DefenderReason>,
    /// 攻撃側の成立した条件(空 = 不成立)。
    pub attacker_reasons: Vec<AttackerReason>,
    pub defender_combat_ready: bool,
    pub attacker_combat_ready: bool,
    /// 双方が成立し、総点で決めた(総点 > 0 なら防御側、それ以外は攻撃側)。
    pub both_met_tie_break: bool,
    pub verdict: Verdict,
}

/// 戦闘可能な師団(人員が 0 でない)が国に残っているか。
pub fn has_combat_ready_divisions(military: &MapMilitary, country: CountryId) -> bool {
    military.divisions().any(|d| d.owner == country && d.combat.manpower > 0)
}

/// 点数を計算する(終結済みは `AlreadyEnded`)。
pub fn war_score(war: &WarRecord, model: &WorldModel, territory: &Territory, military: &MapMilitary, ledger: &BattleLedger) -> Result<WarScoreBreakdown, ScoreError> {
    let summary = summary_of(war, model, territory, military)?;
    score_with_summary(war, &summary, ledger)
}

fn summary_of(war: &WarRecord, model: &WorldModel, territory: &Territory, military: &MapMilitary) -> Result<StateControlSummary, ScoreError> {
    if war.end.is_some() {
        return Err(ScoreError::AlreadyEnded(war.id));
    }
    let owners: BTreeMap<MapStateId, CountryId> = territory.owners().collect();
    summarize_control(model, &owners, military.control()).map_err(ScoreError::Control)
}

fn check_war(war: &WarRecord, summary: &StateControlSummary) -> Result<(), ScoreError> {
    if war.end.is_some() {
        return Err(ScoreError::AlreadyEnded(war.id));
    }
    if summary.state(war.target_state).is_none() {
        return Err(ScoreError::UnknownTarget { war: war.id, state: war.target_state });
    }
    Ok(())
}

/// 点数を、ステート単位の支配の集計(法的所有を含む)から直接計算する(`war_score` の本体。合成した所属表での検証にも使う)。
pub fn score_with_summary(war: &WarRecord, summary: &StateControlSummary, ledger: &BattleLedger) -> Result<WarScoreBreakdown, ScoreError> {
    check_war(war, summary)?;
    let (a, d) = (war.attacker.primary, war.defender.primary);
    let target_held = summary.state(war.target_state).map(|s| s.kind()) == Some(StateControlKind::Full(a));
    let war_goal_score = if target_held { TARGET_POINTS } else { 0 };
    let attacker_occupation = occupation_rate(summary, a, d)?;
    let defender_occupation = occupation_rate(summary, d, a)?;
    let attacker_occupation_score = attacker_occupation.points()?;
    let defender_occupation_score = defender_occupation.points()?;
    let (aw, dw) = ledger.wins(war.id);
    let diff = aw as i64 - dw as i64;
    let battle_score = (diff * BATTLE_POINTS_PER_WIN as i64).clamp(-(BATTLE_POINTS_CAP as i64), BATTLE_POINTS_CAP as i64) as i32;
    let raw_total = war_goal_score + attacker_occupation_score - defender_occupation_score + battle_score;
    let total_score = raw_total.clamp(-TOTAL_POINTS_CAP, TOTAL_POINTS_CAP);
    Ok(WarScoreBreakdown {
        war: war.id,
        target_held,
        war_goal_score,
        attacker_occupation,
        defender_occupation,
        attacker_occupation_score,
        defender_occupation_score,
        attacker_battle_wins: aw,
        defender_battle_wins: dw,
        battle_score,
        raw_total,
        total_score,
    })
}

/// 点数と降伏を判定する(終結済みは `AlreadyEnded`)。読み取り専用で、講和は適用しない。
pub fn assess_war(war: &WarRecord, model: &WorldModel, territory: &Territory, military: &MapMilitary, ledger: &BattleLedger) -> Result<WarAssessment, ScoreError> {
    let summary = summary_of(war, model, territory, military)?;
    assess_with_summary(war, &summary, military, ledger)
}

/// 点数と降伏を、ステート単位の支配の集計から直接判定する(`assess_war` の本体)。師団は `military` から読む。
pub fn assess_with_summary(war: &WarRecord, summary: &StateControlSummary, military: &MapMilitary, ledger: &BattleLedger) -> Result<WarAssessment, ScoreError> {
    let breakdown = score_with_summary(war, summary, ledger)?;
    let (a, d) = (war.attacker.primary, war.defender.primary);
    let defender_combat_ready = has_combat_ready_divisions(military, d);
    let attacker_combat_ready = has_combat_ready_divisions(military, a);

    let mut defender_reasons = Vec::new();
    if breakdown.target_held {
        if breakdown.attacker_occupation.reaches_surrender_rate()? {
            defender_reasons.push(DefenderReason::TargetHeldAndOccupation);
        }
        if !defender_combat_ready {
            defender_reasons.push(DefenderReason::TargetHeldAndNoCombatReadyDivisions);
        }
    }

    let mut attacker_reasons = Vec::new();
    if breakdown.defender_occupation.reaches_surrender_rate()? {
        attacker_reasons.push(AttackerReason::OccupationRate);
    }
    if !attacker_combat_ready {
        let held: Vec<MapStateId> = summary.states().iter().filter(|s| s.legal_owner == Some(a) && s.kind() == StateControlKind::Full(d)).map(|s| s.state).collect();
        if !held.is_empty() {
            attacker_reasons.push(AttackerReason::NoCombatReadyDivisionsAndStateHeld(held));
        }
    }

    let (d_met, a_met) = (!defender_reasons.is_empty(), !attacker_reasons.is_empty());
    let both_met_tie_break = d_met && a_met;
    let verdict = match (d_met, a_met) {
        (false, false) => Verdict::None,
        (true, false) => Verdict::DefenderSurrenders,
        (false, true) => Verdict::AttackerSurrenders,
        (true, true) => {
            if breakdown.total_score > 0 {
                Verdict::DefenderSurrenders
            } else {
                Verdict::AttackerSurrenders
            }
        }
    };
    Ok(WarAssessment { breakdown, defender_reasons, attacker_reasons, defender_combat_ready, attacker_combat_ready, both_met_tie_break, verdict })
}

/// 稼働中の戦争すべてを `MapWarId` 昇順で判定する(終結済みは含めない)。
pub fn assess_active_wars(conflicts: &Conflicts, model: &WorldModel, territory: &Territory, military: &MapMilitary, ledger: &BattleLedger) -> Vec<Result<WarAssessment, ScoreError>> {
    conflicts.active_wars().map(|w| assess_war(w, model, territory, military, ledger)).collect()
}
