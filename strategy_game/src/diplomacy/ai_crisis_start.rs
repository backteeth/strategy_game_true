//! P21-017: AI国家が既存の有効なClaimを月次で評価し、条件を満たせば自らCrisisを
//! 開始する。AIによる新規Claim生成は対象外(既存Claimのみを使用する)。
//!
//! 開始判断は乱数を使わない純粋関数([`compute_power_ratio`]・候補ソート)で行い、
//! 実際の状態変更は既存の共有ドメインAPI(`CrisisRegistry::can_start_crisis`/
//! `start_crisis`、P21-010/011からプレイヤーUIも使う同一関数)へ委ねる。
//!
//! `CrisisRegistry::start_crisis`は呼び出した時点で即座に`CrisisPhase::DemandSent`へ
//! 遷移し要求期限を確定する(P21-011の既存仕様: 「最後通牒として即座にDemandSentへ
//! 入る」)。つまりこのコードベースには「Crisis開始」と「要求送信」を分ける独立した
//! ドメインAPIが存在せず、両者は同一の`start_crisis`呼び出しで不可分に発生する。
//! そのためP21-017の「AI開始Crisisで、送信可能になった要求を日次処理で自動送信する」
//! という要求は、月次評価の中で`start_crisis`を直接呼ぶことで(要求が創出と同時に
//! 送信されるため、日次tickを待たせるより早く)自動的に満たされる — 新しい
//! 「未送信」状態や別の送信専用APIを追加することは、既存APIの実際の形と矛盾する
//! ため行わない(仕様書冒頭の「既存APIの名前が仕様書と異なる場合は実コードを正とし」
//! という指示に従う)。

use crate::app::time::{GameDate, MonthChangedMessage};
use crate::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId};
use crate::country::power::CountryPowerRegistry;
use crate::country::{CountryRegistry, PlayerCountry};
use crate::diplomacy::claims::{ClaimRegistry, ClaimStatus};
use crate::diplomacy::crisis::{CrisisRegistry, diplomatic_conditions_permit_crisis_start};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// 対象国への関係値がこの値以下ならAIはCrisis開始の候補にできる(境界値含む)。
pub const AI_CRISIS_MAX_OPINION: f32 = -25.0;
/// 自国/対象国の国家総合力比がこの値以上ならAIはCrisis開始の候補にできる(境界値含む)。
pub const AI_CRISIS_MIN_POWER_RATIO: f32 = 0.75;

/// §4.1: 関係値だけに基づく開始許可判定(純粋関数)。
pub fn ai_crisis_opinion_permits_start(opinion: f32) -> bool {
    opinion <= AI_CRISIS_MAX_OPINION
}

/// §4.2: 自国/対象国の国家総合力比を計算する(純粋関数)。0除算・NaN・無限大を
/// 一切生成しない。`target_score == 0.0 && initiator_score > 0.0`の場合は
/// (定義上あらゆる閾値を満たす)有限の最大値`f32::MAX`を返す — 真の無限大では
/// ないため`.is_finite()`を満たしたまま、ソート時にも常に他のどんな実比率より
/// 大きい値として扱える。
pub fn compute_power_ratio(initiator_score: f32, target_score: f32) -> f32 {
    if target_score > 0.0 {
        initiator_score / target_score
    } else if initiator_score > 0.0 {
        f32::MAX
    } else {
        1.0
    }
}

/// §4.2: 国力比だけに基づく開始許可判定(純粋関数)。
pub fn ai_crisis_power_ratio_permits_start(ratio: f32) -> bool {
    ratio >= AI_CRISIS_MIN_POWER_RATIO
}

/// AIによるCrisis開始候補となる、資格条件(§3)を満たした1件のClaim。
#[derive(Debug, Clone, Copy, PartialEq)]
struct AiCrisisCandidate {
    claim_id: ClaimId,
    claimant: CountryId,
    target: CountryId,
    target_state: StateId,
    opinion: f32,
    power_ratio: f32,
}

/// §5: 複数候補からの選択順位。(1)関係値が低いもの優先 (2)power ratioが高いもの優先
/// (3)Claimに有効期限が存在しないため省略 (4)Claim ID昇順 (5)対象CountryId昇順
/// (6)対象StateId昇順。浮動小数点比較は`partial_cmp`が`None`(NaNとの比較、実際には
/// 発生しない)を返してもpanicしないよう`Equal`へフォールバックする。
fn compare_candidates(a: &AiCrisisCandidate, b: &AiCrisisCandidate) -> Ordering {
    a.opinion
        .partial_cmp(&b.opinion)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            b.power_ratio
                .partial_cmp(&a.power_ratio)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| a.claim_id.0.cmp(&b.claim_id.0))
        .then_with(|| a.target.0.cmp(&b.target.0))
        .then_with(|| a.target_state.0.cmp(&b.target_state.0))
}

/// 現在戦争中の国家ペアを正規化(`CountryId`昇順)して表す。
pub(crate) type WarPair = (CountryId, CountryId);

fn normalize_pair(a: CountryId, b: CountryId) -> WarPair {
    if a.0 <= b.0 { (a, b) } else { (b, a) }
}

/// P21-017(二次レビュー対応・§3の性能指摘への対応): `WarRegistry::
/// are_countries_at_war`をClaim/候補単位で繰り返し呼ぶと、呼び出し回数
/// (=`unique claimant-target pairs`)× O(active wars)になり得るため、月次評価の
/// 冒頭で現在Active な全Warの攻撃側×防御側の組を一度だけSetへ集約し、以後は
/// O(1)参照にする。`are_countries_at_war(a,b)`の定義
/// (`(attackers.contains(a) && defenders.contains(b)) || 逆`)と完全に同値。
pub(crate) fn build_at_war_pairs(war_registry: &WarRegistry) -> HashSet<WarPair> {
    let mut pairs = HashSet::new();
    for war in war_registry.wars.values() {
        if war.status != WarStatus::Active {
            continue;
        }
        for &a in &war.attackers {
            for &b in &war.defenders {
                pairs.insert(normalize_pair(a, b));
            }
        }
    }
    pairs
}

/// §13: `countries × claims`/`claims × crises`の総当たりを避けるため、Crisis・
/// 正当化・Warを一回ずつ走査してdangling/重複判定用の集合を事前構築してから、
/// Claim Registryを一回だけ走査する。国家ペアごとの資格判定(同盟・戦争・条約・
/// 国力・関係値)は`(claimant, target)`単位でメモ化し、同じ対象国へ複数Claimを持つ
/// claimantがいても再計算しない。
///
/// 戻り値は claimant ごとにグループ化され、各Vecは`compare_candidates`で
/// 既にソート済み(先頭が最優先候補)。
#[allow(clippy::too_many_arguments)]
fn collect_ai_crisis_candidates(
    claim_registry: &ClaimRegistry,
    crisis_registry: &CrisisRegistry,
    justification_registry: &WarJustificationRegistry,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    war_registry: &WarRegistry,
    power_registry: &CountryPowerRegistry,
    player_country: Option<CountryId>,
    current_date: &str,
) -> HashMap<CountryId, Vec<AiCrisisCandidate>> {
    // 1. 進行中CrisisのClaim IDと開始国を一回だけ収集する(§13手順1)。
    let mut claims_with_unresolved_crisis: HashSet<ClaimId> = HashSet::new();
    let mut claimants_with_unresolved_self_crisis: HashSet<CountryId> = HashSet::new();

    for crisis in crisis_registry.crises.values() {
        let unresolved = crate::diplomacy::crisis::is_unresolved_phase(crisis.current_phase);
        if let Some(claim_id) = crisis.related_claim_id
            && unresolved
        {
            claims_with_unresolved_crisis.insert(claim_id);
        }
        if unresolved {
            claimants_with_unresolved_self_crisis.insert(crisis.initiator);
        }
    }

    // 2. 出所を問わず、既存の全正当化から (initiator, target, target_state) の
    //    組を一回だけ収集する(§13手順2、二次レビュー対応)。以前はCrisis由来の
    //    正当化(`source_crisis_id`→`related_claim_id`で追跡できるもの)しか
    //    除外しておらず、`country_ai::process_war_preparation_ai`が独立に開始する
    //    生の正当化との並行進行を防げていなかった。正当化の出所を問わず
    //    「この(claimant, target, target_state)は既に正当化への道筋がある」を
    //    直接見ることで、Crisis由来かどうかに依存しない、より単純かつ正確な
    //    判定になる。
    let justified_triples: HashSet<(CountryId, CountryId, StateId)> = justification_registry
        .justifications
        .values()
        .map(|j| (j.initiator, j.target, j.target_state))
        .collect();

    // 3. 現在Active な戦争中ペアを一回だけ集約する(§13手順3相当、二次レビュー対応)。
    let at_war_pairs = build_at_war_pairs(war_registry);

    let country_by_id: HashMap<CountryId, &crate::country::CountryData> = country_registry
        .countries
        .iter()
        .map(|c| (c.id, c))
        .collect();

    // (claimant, target)単位でのメモ化キャッシュ: `None`は資格なし、
    // `Some((opinion, ratio))`は候補として使う値。
    let mut pair_cache: HashMap<(CountryId, CountryId), Option<(f32, f32)>> = HashMap::new();

    // 4. Claim Registryを一回だけ走査する(§13手順4)。
    let mut grouped: HashMap<CountryId, Vec<AiCrisisCandidate>> = HashMap::new();

    for claim in claim_registry.claims.values() {
        if claim.status != ClaimStatus::Active {
            continue;
        }
        if claims_with_unresolved_crisis.contains(&claim.id) {
            continue;
        }

        let claimant = claim.claimant_country;
        if Some(claimant) == player_country {
            continue;
        }
        if claimants_with_unresolved_self_crisis.contains(&claimant) {
            continue;
        }
        // P21-021: 消滅済み(Eliminated)国家はCrisis開始の要求国/対象国いずれにも
        // なり得ない(§12: is_active()をAI列挙点で共有する)。claimant側は
        // Claim自体がelimination清掃で`Invalidated`へ遷移するため構造的にも
        // ここへ到達しないが、明示的にも確認する。
        if !country_by_id.get(&claimant).is_some_and(|c| c.is_active()) {
            continue;
        }

        let Some(state) = state_registry.get(claim.target_state) else {
            continue;
        };
        let target = state.owner_country_id;
        if target == claimant || !country_by_id.get(&target).is_some_and(|c| c.is_active()) {
            continue;
        }

        if justified_triples.contains(&(claimant, target, claim.target_state)) {
            continue;
        }

        let pair_key = (claimant, target);
        let cached = *pair_cache.entry(pair_key).or_insert_with(|| {
            evaluate_pair_eligibility(
                claimant,
                target,
                diplomacy_registry,
                &at_war_pairs,
                power_registry,
                current_date,
            )
        });

        let Some((opinion, power_ratio)) = cached else {
            continue;
        };

        grouped
            .entry(claimant)
            .or_default()
            .push(AiCrisisCandidate {
                claim_id: claim.id,
                claimant,
                target,
                target_state: claim.target_state,
                opinion,
                power_ratio,
            });
    }

    for candidates in grouped.values_mut() {
        candidates.sort_by(compare_candidates);
    }

    grouped
}

/// `(claimant, target)`ペア単位の資格判定(§3項目7-9,13-15)。国力評価が
/// 片方でも欠落している場合はpanicせず資格なし(`None`)として扱う(§3項目24-25)。
///
/// P21-018: `pub(crate)`化し`diplomacy::ai_claim_generation`からも同じ関数を呼ぶ
/// (opinion/power ratio/同盟/NAP/休戦/戦争中の判定基準をCrisis開始とClaim生成とで
/// 二重実装しないため)。
///
/// P21-017(二次レビュー対応): 同盟・不可侵条約・休戦の判定は
/// [`diplomatic_conditions_permit_crisis_start`](プレイヤーUIが呼ぶ
/// `CrisisRegistry::can_start_crisis`と全く同じ関数)を直接呼び、AI候補選定と
/// 共有ドメインAPIの判定基準が将来ズレることを防ぐ。「現在戦争中」だけは、
/// 呼び出し元(このモジュール)が事前集約した`at_war_pairs`をO(1)参照する
/// (`WarRegistry::are_countries_at_war`をclaim単位で繰り返し呼ぶとO(pairs×wars)
/// になるため)。
pub(crate) fn evaluate_pair_eligibility(
    claimant: CountryId,
    target: CountryId,
    diplomacy_registry: &DiplomacyRegistry,
    at_war_pairs: &HashSet<WarPair>,
    power_registry: &CountryPowerRegistry,
    current_date: &str,
) -> Option<(f32, f32)> {
    if at_war_pairs.contains(&normalize_pair(claimant, target)) {
        return None;
    }
    if diplomatic_conditions_permit_crisis_start(claimant, target, diplomacy_registry, current_date)
        .is_err()
    {
        return None;
    }

    let initiator_assessment = power_registry.get(claimant)?;
    let target_assessment = power_registry.get(target)?;

    let opinion = diplomacy_registry.get_or_default(claimant, target).opinion;
    if !ai_crisis_opinion_permits_start(opinion) {
        return None;
    }

    let power_ratio = compute_power_ratio(
        initiator_assessment.total_score,
        target_assessment.total_score,
    );
    if !ai_crisis_power_ratio_permits_start(power_ratio) {
        return None;
    }

    Some((opinion, power_ratio))
}

/// 候補一覧(既にソート済み)を先頭から順に試し、既存の`can_start_crisis`
/// (`start_crisis`が内部で呼ぶ)による直前の再検証に最初に成功したものを開始する。
/// 全候補が失敗した場合は何も開始しない(`None`)。1回の呼び出しで最大1件しか
/// 開始しない(§5末尾: 「成功する新規Crisisは一回の月次評価につき一件まで」)。
#[allow(clippy::too_many_arguments)]
fn try_start_crisis_from_candidates(
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &ClaimRegistry,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    war_registry: &WarRegistry,
    candidates: &[AiCrisisCandidate],
    current_date: &str,
) -> Option<DiplomaticCrisisId> {
    for candidate in candidates {
        let claim = claim_registry.claims.get(&candidate.claim_id)?;
        if let Ok(id) = crisis_registry.start_crisis(
            claim,
            candidate.claimant,
            candidate.target,
            current_date.to_string(),
            country_registry,
            state_registry,
            diplomacy_registry,
            war_registry,
        ) {
            return Some(id);
        }
    }
    None
}

/// P21-017のエントリーポイント(Bevy非依存)。AI国家をCountryId昇順で処理し、
/// 各国最大1件までCrisisを開始する。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_ai_crisis_start(
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &ClaimRegistry,
    justification_registry: &WarJustificationRegistry,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    war_registry: &WarRegistry,
    power_registry: &CountryPowerRegistry,
    player_country: &PlayerCountry,
    current_date: &str,
) {
    let grouped = collect_ai_crisis_candidates(
        claim_registry,
        crisis_registry,
        justification_registry,
        country_registry,
        state_registry,
        diplomacy_registry,
        war_registry,
        power_registry,
        player_country.0,
        current_date,
    );

    let mut claimants: Vec<CountryId> = grouped.keys().copied().collect();
    claimants.sort_by_key(|c| c.0);

    for claimant in claimants {
        let candidates = &grouped[&claimant];
        try_start_crisis_from_candidates(
            crisis_registry,
            claim_registry,
            country_registry,
            state_registry,
            diplomacy_registry,
            war_registry,
            candidates,
            current_date,
        );
    }
}

/// §6: `evaluate_ai_crisis_start`が実際に何回呼ばれたかを直接検証するための
/// 診断専用Resource。[`crate::diplomacy::update::AiSupportEvaluationCount`]
/// (P21-015)と全く同じ設計: `Option<ResMut<_>>`として受け取るため、Worldに
/// 存在しなければ静かに`None`となり本番では一切のオーバーヘッドを生まない。
/// テストが明示的に`insert_resource`した場合にのみ増分され、「同一フレームに
/// 複数の`MonthChangedMessage`があっても評価はフレームにつき最大1回」という
/// 要件を、結果だけでなく実際の呼び出し回数そのもので検証できるようにする。
#[derive(Resource, Default, Debug)]
pub struct AiCrisisStartEvaluationCount(pub usize);

/// System: `MonthChangedMessage`が発生したフレームにつき最大1回だけ
/// `evaluate_ai_crisis_start`を実行する。
#[allow(clippy::too_many_arguments)]
pub fn handle_monthly_ai_crisis_start(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut crisis_registry: ResMut<CrisisRegistry>,
    claim_registry: Res<ClaimRegistry>,
    justification_registry: Res<WarJustificationRegistry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    diplomacy_registry: Res<DiplomacyRegistry>,
    war_registry: Res<WarRegistry>,
    power_registry: Res<CountryPowerRegistry>,
    player_country: Res<PlayerCountry>,
    date: Res<GameDate>,
    mut eval_counter: Option<ResMut<AiCrisisStartEvaluationCount>>,
) {
    let mut evaluated_this_frame = false;
    for _event in month_events.read() {
        if evaluated_this_frame {
            continue;
        }
        evaluated_this_frame = true;

        evaluate_ai_crisis_start(
            &mut crisis_registry,
            &claim_registry,
            &justification_registry,
            &country_registry,
            &state_registry,
            &diplomacy_registry,
            &war_registry,
            &power_registry,
            &player_country,
            &date.display(),
        );

        if let Some(counter) = eval_counter.as_mut() {
            counter.0 += 1;
        }
    }
}

#[cfg(test)]
mod tests;
