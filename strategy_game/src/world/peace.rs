//! P32-044 (M7-d): 講和の提案・受諾と、日次の自動講和(純粋な Rust。Bevy 非依存)。
//!
//! **シナリオの戦争・所有・支配は検証入力であり、正式な世界データではない。** 範囲は提案・受諾と降伏後の自動講和まで。
//! 前線消失・長期停滞による白紙講和は M7-d の後続項目(入力の調査は P32-044 報告書 §6)、勝敗表示と GUI は M7-e、保存は M8、本体への統合は M9。
//!
//! ## 提案と受諾
//! [`PeaceProposal`] = 戦争 ID・提案国・受領国・条件・提案日。提案は存続中の**対立する代表国どうし**(攻撃側代表国と防御側代表国)の間でだけ作れる。
//! - [`ProposalTerms::WhitePeace`]: どちらの代表国が提案してもよい。受諾時に、開戦から [`WHITE_PEACE_MIN_WAR_DAYS`] 日以上(`今日 − 開始日 ≥ 30`)かつ、
//!   **攻撃側視点**の総点が [`WHITE_PEACE_MIN_SCORE`] 以上(提案国によらず視点は共通)。領土移転なし。
//! - [`ProposalTerms::CedeTarget`]: 攻撃側代表国 → 防御側代表国。防御側代表国が目標の法的所有者で、総点が [`CESSION_MIN_SCORE`] 以上、
//!   **または防御側単独の降伏条件が成立**(M7-c の `defender_reasons` が空でないこと。双方成立時の最終裁定 `verdict` ではない)。
//! - [`ProposalTerms::AcceptDefeat`]: 攻撃側代表国 → 防御側代表国。領土移転なしで防御側の勝利。点数・日数の条件はない。
//!
//! 照会 [`check_acceptance`] は読み取り専用で、受諾したときの結果(`plan_end_war` の計画)まで返す。受諾 [`accept_peace`] は
//! **受諾の時点の**日付・参加国(存続)・法的所有・戦争点数・降伏条件を再検証し、既存の `plan_end_war` / `apply_end_war` を通す。
//! 拒否(未知・終結済みの戦争、開戦前、権限のない国、受領国でない国、前提の失効、適用の失敗)は、**状態を変えずに**理由つきで返す。
//! 二重の受諾は、戦争が終結済みなので `WarEnded` で拒否され、割譲・休戦は二重に適用されない。
//!
//! ## 日次の自動講和([`run_auto_peace`])
//! `advance_day` の**終了後**に呼ぶ。1) その日の確定した戦闘(`BattleFinished`)を `BattleLedger` に集計(講和より先)、
//! 2) 開始時に稼働中だった戦争を `MapWarId` 昇順に、**各適用の直前に現在の状態を読み直して**降伏を判定(M7-c)する。
//! 先行する割譲・国家消滅で連鎖終結した戦争は処理済みとして飛ばす。防御側の降伏 = 目標の割譲と攻撃側の勝利(`EndReason::DefenderSurrender`)、
//! 攻撃側の降伏 = 領土移転なしの防御側の勝利(`EndReason::AttackerSurrender`)。提案の受諾条件(30 日・点数)は介さず、共通の終結 API を通す。
//! 適用に失敗した講和(別戦争で敵対が続く組の戦闘が目標ステート内にある、首都候補がない、など)は戦争を稼働中のまま残し、理由を報告して次回に再判定する。
//!
//! 休戦は M7-b の終結 API が付ける(直接成立した講和は降伏を含めて、存続する代表国間に終結日から 1825 日。連鎖終結には新設しない)。

use super::connectivity::ProvinceGraph;
use super::model::WorldModel;
use super::movement::{MapMilitary, MoveEvent};
use super::territory::{Territory, TransferContext};
use super::war_record::{Conflicts, EndWarApplyError, EndWarError, EndWarOutcome, EndWarPlan, EndWarRequest, MapWarId, WarEnd, apply_end_war, end_war, plan_end_war};
use super::war_score::{BattleLedger, LedgerEntry, ScoreError, Verdict, WarAssessment, assess_war};
use crate::common::CountryId;

/// 白紙講和に必要な開戦からの日数(以上)。
pub const WHITE_PEACE_MIN_WAR_DAYS: u32 = 30;
/// 白紙講和に必要な、攻撃側視点の総点の下限(以上)。
pub const WHITE_PEACE_MIN_SCORE: i32 = -10;
/// 割譲を受諾できる総点の下限(以上。防御側単独の降伏条件が成立していれば点数は問わない)。
pub const CESSION_MIN_SCORE: i32 = 60;

/// 提案の条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalTerms {
    WhitePeace,
    CedeTarget,
    AcceptDefeat,
}

/// 講和の提案(値。受諾までの間に状態が変わっても、受諾時に再検証する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeaceProposal {
    pub war: MapWarId,
    pub proposer: CountryId,
    pub recipient: CountryId,
    pub terms: ProposalTerms,
    /// 提案日(日番号)。
    pub day: u32,
}

/// 提案・受諾の拒否(どれも状態を変えない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeaceError {
    UnknownWar(MapWarId),
    /// 終結済み(二重の受諾を含む)。
    WarEnded { war: MapWarId, end: WarEnd },
    /// 日が開戦より前。
    BeforeWarStart { war: MapWarId, start_day: u32, day: u32 },
    /// 受諾の日が提案日より前。
    DayBeforeProposal { proposal_day: u32, today: u32 },
    /// 存続していない国(消滅、または国家の一覧にない)。
    CountryNotActive(CountryId),
    /// 提案国・受領国がこの戦争の対立する代表国の組ではない。
    NotOpposingPrimaries { war: MapWarId, proposer: CountryId, recipient: CountryId },
    /// 条件に対して提案国の権限がない(割譲・敗北は攻撃側代表国だけ)。
    NotAuthorized { terms: ProposalTerms, proposer: CountryId },
    /// 受諾する国が指定された受領国ではない。
    NotRecipient { recipient: CountryId, acceptor: CountryId },
    /// 白紙講和: 開戦からの日数が足りない。
    WhitePeaceTooEarly { war_days: u32, required: u32 },
    /// 白紙講和: 攻撃側視点の総点が下限に届かない。
    WhitePeaceScoreTooLow { score: i32, required: i32 },
    /// 割譲: 防御側代表国が目標の法的所有者ではない。
    CedeNotTargetOwner { owner: Option<CountryId> },
    /// 割譲: 総点も防御側単独の降伏条件も満たさない。
    CedeConditionsNotMet { score: i32, required: i32 },
    /// 点数・降伏の判定に失敗した。
    Score(ScoreError),
    /// 講和の計画が成り立たない(首都候補がない、戦闘の中止指定が必要、など)。
    Plan(EndWarError),
    /// 計画の適用の直前の再検証で拒否された。
    Apply(EndWarApplyError),
}

impl std::fmt::Display for PeaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeaceError::UnknownWar(w) => write!(f, "{w} は存在しません"),
            PeaceError::WarEnded { war, end } => write!(f, "{war} は {} 日に終結済みです", end.day),
            PeaceError::BeforeWarStart { war, start_day, day } => write!(f, "{war} の開始日 {start_day} より前の日 {day} です"),
            PeaceError::DayBeforeProposal { proposal_day, today } => write!(f, "受諾の日 {today} が提案日 {proposal_day} より前です"),
            PeaceError::CountryNotActive(c) => write!(f, "CountryId {} は存続していません", c.0),
            PeaceError::NotOpposingPrimaries { war, proposer, recipient } => write!(f, "CountryId {} と {} は {war} の対立する代表国の組ではありません", proposer.0, recipient.0),
            PeaceError::NotAuthorized { terms, proposer } => write!(f, "CountryId {} は {terms:?} を提案する権限がありません(割譲・敗北は攻撃側代表国だけ)", proposer.0),
            PeaceError::NotRecipient { recipient, acceptor } => write!(f, "受領国は CountryId {} で、CountryId {} は受諾できません", recipient.0, acceptor.0),
            PeaceError::WhitePeaceTooEarly { war_days, required } => write!(f, "白紙講和には開戦から {required} 日以上が必要です(現在 {war_days} 日)"),
            PeaceError::WhitePeaceScoreTooLow { score, required } => write!(f, "白紙講和には攻撃側視点の総点が {required} 以上必要です(現在 {score})"),
            PeaceError::CedeNotTargetOwner { owner } => match owner {
                Some(o) => write!(f, "防御側代表国は目標の法的所有者ではありません(所有者 CountryId {})", o.0),
                None => write!(f, "目標に法的所有者がありません"),
            },
            PeaceError::CedeConditionsNotMet { score, required } => write!(f, "割譲には総点 {required} 以上、または防御側単独の降伏条件が必要です(総点 {score}、降伏条件は不成立)"),
            PeaceError::Score(e) => write!(f, "点数・降伏の判定に失敗しました: {e}"),
            PeaceError::Plan(e) => write!(f, "講和を適用できません: {e}"),
            PeaceError::Apply(e) => write!(f, "講和の適用の直前の再検証で拒否されました: {e}"),
        }
    }
}

impl std::error::Error for PeaceError {}

/// 戦争の存在・稼働・日付・提案国と受領国の資格(状態を変えない)。
fn validate_standing(conflicts: &Conflicts, territory: &Territory, p: &PeaceProposal, today: u32) -> Result<(), PeaceError> {
    let war = conflicts.war(p.war).ok_or(PeaceError::UnknownWar(p.war))?;
    if let Some(end) = war.end {
        return Err(PeaceError::WarEnded { war: war.id, end });
    }
    if today < war.start_day {
        return Err(PeaceError::BeforeWarStart { war: war.id, start_day: war.start_day, day: today });
    }
    if today < p.day {
        return Err(PeaceError::DayBeforeProposal { proposal_day: p.day, today });
    }
    for c in [p.proposer, p.recipient] {
        if !territory.is_active(c) {
            return Err(PeaceError::CountryNotActive(c));
        }
    }
    let (a, d) = (war.attacker.primary, war.defender.primary);
    let opposing = (p.proposer == a && p.recipient == d) || (p.proposer == d && p.recipient == a);
    if !opposing {
        return Err(PeaceError::NotOpposingPrimaries { war: war.id, proposer: p.proposer, recipient: p.recipient });
    }
    if matches!(p.terms, ProposalTerms::CedeTarget | ProposalTerms::AcceptDefeat) && p.proposer != a {
        return Err(PeaceError::NotAuthorized { terms: p.terms, proposer: p.proposer });
    }
    Ok(())
}

/// 提案を作る(読み取り専用。戦争が稼働中で、提案日が開戦以後で、提案国と受領国が存続する対立する代表国の組で、条件に対する権限があること)。
/// 点数・日数の条件は受諾の時点で確かめる([`check_acceptance`])。
pub fn propose_peace(conflicts: &Conflicts, territory: &Territory, war: MapWarId, proposer: CountryId, recipient: CountryId, terms: ProposalTerms, day: u32) -> Result<PeaceProposal, PeaceError> {
    let p = PeaceProposal { war, proposer, recipient, terms, day };
    validate_standing(conflicts, territory, &p, day)?;
    Ok(p)
}

/// 受諾できる場合の結果(照会の戻り値)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acceptance {
    /// 受諾の時点の点数・降伏判定。
    pub assessment: WarAssessment,
    /// 受諾したときに実行する終結の要求。
    pub request: EndWarRequest,
    /// 適用した場合の結果の計画(`plan_end_war`)。
    pub plan: EndWarPlan,
}

/// 受諾の可否の照会(読み取り専用)。受諾の時点(`today`)の状態で全条件を検証し、通れば適用の計画まで返す。
#[allow(clippy::too_many_arguments)]
pub fn check_acceptance(
    ctx: TransferContext<'_>,
    territory: &Territory,
    conflicts: &Conflicts,
    military: &MapMilitary,
    ledger: &BattleLedger,
    proposal: &PeaceProposal,
    acceptor: CountryId,
    today: u32,
) -> Result<Acceptance, PeaceError> {
    validate_standing(conflicts, territory, proposal, today)?;
    if acceptor != proposal.recipient {
        return Err(PeaceError::NotRecipient { recipient: proposal.recipient, acceptor });
    }
    let war = conflicts.war(proposal.war).ok_or(PeaceError::UnknownWar(proposal.war))?;
    let assessment = assess_war(war, ctx.model, territory, military, ledger).map_err(PeaceError::Score)?;
    let score = assessment.breakdown.total_score;
    let request = match proposal.terms {
        ProposalTerms::WhitePeace => {
            let war_days = today - war.start_day;
            if war_days < WHITE_PEACE_MIN_WAR_DAYS {
                return Err(PeaceError::WhitePeaceTooEarly { war_days, required: WHITE_PEACE_MIN_WAR_DAYS });
            }
            if score < WHITE_PEACE_MIN_SCORE {
                return Err(PeaceError::WhitePeaceScoreTooLow { score, required: WHITE_PEACE_MIN_SCORE });
            }
            EndWarRequest::white_peace(war.id, today)
        }
        ProposalTerms::CedeTarget => {
            let owner = territory.owner(war.target_state);
            if owner != Some(war.defender.primary) {
                return Err(PeaceError::CedeNotTargetOwner { owner });
            }
            // 防御側**単独**の条件(`defender_reasons`)。双方成立時の最終裁定(`verdict`)とは別。
            if score < CESSION_MIN_SCORE && assessment.defender_reasons.is_empty() {
                return Err(PeaceError::CedeConditionsNotMet { score, required: CESSION_MIN_SCORE });
            }
            EndWarRequest::cession(war.id, today)
        }
        ProposalTerms::AcceptDefeat => EndWarRequest::defeat(war.id, today),
    };
    let plan = plan_end_war(ctx, territory, conflicts, military, &request).map_err(PeaceError::Plan)?;
    Ok(Acceptance { assessment, request, plan })
}

/// 提案を受諾して講和を適用する。受諾の時点で再検証し(`check_acceptance`)、計画を `apply_end_war` で適用する。拒否は状態を変えない。
#[allow(clippy::too_many_arguments)]
pub fn accept_peace(
    ctx: TransferContext<'_>,
    territory: &mut Territory,
    conflicts: &mut Conflicts,
    military: &mut MapMilitary,
    ledger: &BattleLedger,
    proposal: &PeaceProposal,
    acceptor: CountryId,
    today: u32,
) -> Result<EndWarOutcome, PeaceError> {
    let acc = check_acceptance(ctx, territory, conflicts, military, ledger, proposal, acceptor, today)?;
    apply_end_war(ctx, territory, conflicts, military, &acc.plan).map_err(PeaceError::Apply)
}

// ---------------------------------------------------------------- 日次の自動講和

/// 自動講和の1件の記録。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoPeaceEntry {
    /// 降伏で終結した(連鎖で終結した戦争は `outcome.ended` に入る)。
    Ended { war: MapWarId, verdict: Verdict, outcome: Box<EndWarOutcome> },
    /// 先行する講和の連鎖で終結していたので処理済みとして飛ばした。
    SkippedEnded { war: MapWarId },
    /// 降伏の条件は成立したが、講和の適用に失敗した(戦争は稼働中のまま。次回に再判定)。
    Failed { war: MapWarId, verdict: Verdict, error: EndWarApplyError },
    /// 点数・降伏の判定ができなかった(戦争は稼働中のまま)。
    NotJudged { war: MapWarId, error: ScoreError },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoPeaceReport {
    /// その日の確定した戦闘の集計(`MapBattleId` 昇順)。
    pub ledger_entries: Vec<LedgerEntry>,
    /// 降伏の判定が出た戦争・飛ばした戦争の記録(`MapWarId` 昇順)。降伏しない戦争は載せない。
    pub entries: Vec<AutoPeaceEntry>,
}

/// `advance_day` の終了後の自動講和。`events` はその日の `advance_day` の戻り値、`day` はその日の日番号(終結日・休戦の起点)。
pub fn run_auto_peace(
    ctx: TransferContext<'_>,
    territory: &mut Territory,
    conflicts: &mut Conflicts,
    military: &mut MapMilitary,
    ledger: &mut BattleLedger,
    events: &[MoveEvent],
    day: u32,
) -> AutoPeaceReport {
    // 1) その日の確定した戦闘を、講和より先に集計する(集計は講和の適用前の戦争の記録に対して行う)
    let ledger_entries = ledger.record_events(conflicts, events);
    // 2) 開始時に稼働中だった戦争を ID 昇順に。各戦争の判定・適用の直前に現在の状態を読み直す
    let ids: Vec<MapWarId> = conflicts.active_wars().map(|w| w.id).collect();
    let mut entries = Vec::new();
    for id in ids {
        let Some(war) = conflicts.war(id) else { continue };
        if !war.is_active() {
            entries.push(AutoPeaceEntry::SkippedEnded { war: id });
            continue;
        }
        let assessment = match assess_war(war, ctx.model, territory, military, ledger) {
            Ok(a) => a,
            Err(error) => {
                entries.push(AutoPeaceEntry::NotJudged { war: id, error });
                continue;
            }
        };
        let request = match assessment.verdict {
            Verdict::None => continue,
            Verdict::DefenderSurrenders => EndWarRequest::defender_surrender(id, day),
            Verdict::AttackerSurrenders => EndWarRequest::attacker_surrender(id, day),
        };
        match end_war(ctx, territory, conflicts, military, &request) {
            Ok(outcome) => entries.push(AutoPeaceEntry::Ended { war: id, verdict: assessment.verdict, outcome: Box::new(outcome) }),
            Err(error) => entries.push(AutoPeaceEntry::Failed { war: id, verdict: assessment.verdict, error }),
        }
    }
    AutoPeaceReport { ledger_entries, entries }
}

/// 呼び出し側の便宜: 世界の参照からコンテキストを作る。
pub fn context<'a>(model: &'a WorldModel, graph: &'a ProvinceGraph) -> TransferContext<'a> {
    TransferContext { model, graph }
}
