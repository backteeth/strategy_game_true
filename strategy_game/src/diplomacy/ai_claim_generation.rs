//! P21-018: AI国家が月次で、国境を接する外国州へ新しい領土請求(`TerritorialClaim`)を
//! 生成する。生成したClaimは即座には使わず、次回以降の月変更で
//! `diplomacy::ai_crisis_start`(P21-017)が通常のClaimとして評価する。
//!
//! このモジュール自身はCrisisを開始しない — 「新しいClaimを作る」ことだけが責務であり、
//! 「そのClaimからCrisisを始める」処理は既存のP21-017経路を無条件に再利用する
//! (専用のAI Crisis開始経路を新設しない、という仕様§11の指示どおり)。
//!
//! 開始判断(国境候補の構築・country/state資格判定・候補ソート)は乱数を使わない
//! 純粋関数/決定論的走査で行い、実際の書き込みは既存の共有ドメインAPI
//! (`ClaimRegistry::create_claim`、プレイヤーUIの`diplomacy_panel`も呼ぶ同一関数)へ
//! 委ねる。opinion/power ratioの閾値・判定関数、同盟/NAP/休戦/戦争中の判定、
//! 「Crisisが未解決か」の定義は全て`ai_crisis_start`(P21-017)/`crisis`モジュールの
//! 既存実装を`pub(crate)`化して再利用し、このモジュールで複製しない。

use crate::app::time::{GameDate, MonthChangedMessage};
use crate::common::{ClaimId, CountryId, StateId};
use crate::country::power::CountryPowerRegistry;
use crate::country::{CountryData, CountryRegistry, PlayerCountry};
use crate::diplomacy::ai_crisis_start::evaluate_pair_eligibility;
use crate::diplomacy::claims::{ClaimRegistry, ClaimSource, ClaimStatus};
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::justification::WarJustificationRegistry;
use bevy::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// AI国家が新しいClaimを生成できるようになるまでの、直近取得Claimからの経過月数。
pub const AI_CLAIM_GENERATION_COOLDOWN_MONTHS: i32 = 12;

/// AI生成Claimの強度。`ClaimRegistry::create_claim`が(プレイヤー作成分も含め)常に
/// 内部でこの値を書き込むため、このモジュールから`create_claim`へ引数として渡すことは
/// ない(§8: 「既存に適切な標準値がない場合だけ」新しい定数を使う、の対偶 — 既に標準値が
/// 存在するので複製しない)。テスト・完了報告のためだけにこの定数を公開する。
pub const AI_GENERATED_CLAIM_STRENGTH: f32 = 50.0;

/// `year*12 + (month-1)`形式の単調増加な月indexへ変換する(純粋関数)。
fn month_index(year: i32, month: u8) -> i32 {
    year * 12 + (month as i32 - 1)
}

/// §3: cooldownの充足判定(純粋関数)。`latest_claim_month_index`が`None`(過去にClaimを
/// 一件も持ったことがない)なら常に生成可能。未来日付のClaim(`latest > current`)は
/// `elapsed`が負になり、`AI_CLAIM_GENERATION_COOLDOWN_MONTHS`未満のため自然に生成不可となる
/// (特別扱い不要)。
pub fn claim_generation_cooldown_satisfied(
    latest_claim_month_index: Option<i32>,
    current_month_index: i32,
) -> bool {
    match latest_claim_month_index {
        None => true,
        Some(latest) => current_month_index - latest >= AI_CLAIM_GENERATION_COOLDOWN_MONTHS,
    }
}

/// Claimの`created_date`を月indexへ変換する。解析できない不正な日付文字列の場合は
/// panicせず、`current_month_index`(=経過0か月)を返すことで安全側(cooldown不成立)に
/// フォールバックする — 「不正日付を見つけてもpanicしない」(§3)を満たしつつ、
/// 壊れた日付を持つ国が誤って即座にcooldownを回避できてしまう事態も防ぐ。
fn claim_month_index_or_blocking(created_date: &str, current_month_index: i32) -> i32 {
    GameDate::from_string(created_date)
        .map(|d| month_index(d.year, d.month))
        .unwrap_or(current_month_index)
}

/// ClaimRegistryを1回だけ走査し、(1)現在Active Claimを持つ国の集合と、
/// (2)国ごとの最新Claim取得月index(source/Active・Consumed問わず)を同時に構築する。
fn collect_claim_history(
    claim_registry: &ClaimRegistry,
    current_month_index: i32,
) -> (HashSet<CountryId>, HashMap<CountryId, i32>) {
    let mut claimants_with_active_claim: HashSet<CountryId> = HashSet::new();
    let mut latest_month_index_by_claimant: HashMap<CountryId, i32> = HashMap::new();

    for claim in claim_registry.claims.values() {
        if claim.status == ClaimStatus::Active {
            claimants_with_active_claim.insert(claim.claimant_country);
        }
        let month_idx = claim_month_index_or_blocking(&claim.created_date, current_month_index);
        latest_month_index_by_claimant
            .entry(claim.claimant_country)
            .and_modify(|m| *m = (*m).max(month_idx))
            .or_insert(month_idx);
    }

    (claimants_with_active_claim, latest_month_index_by_claimant)
}

/// WarRegistryを1回だけ走査し、現在Activeな戦争に参加中(攻撃側・防御側いずれか)の
/// 全国家を集合として構築する。
fn collect_war_participants(war_registry: &WarRegistry) -> HashSet<CountryId> {
    let mut participants: HashSet<CountryId> = HashSet::new();
    for war in war_registry.wars.values() {
        if war.status != WarStatus::Active {
            continue;
        }
        participants.extend(war.attackers.iter().copied());
        participants.extend(war.defenders.iter().copied());
    }
    participants
}

/// WarJustificationRegistryを1回だけ走査し、出所を問わず(Crisis由来/生正当化問わず)
/// 進行中またはReadyな正当化を持つ全initiatorを集合として構築する。
fn collect_justification_initiators(
    justification_registry: &WarJustificationRegistry,
) -> HashSet<CountryId> {
    justification_registry
        .justifications
        .values()
        .map(|j| j.initiator)
        .collect()
}

/// §10: 国境候補の構築結果。StateRegistryを1回だけ走査して同時に構築する。
struct BorderData {
    /// 国ごとの法的所有陸州数(0件ならClaim生成資格なし、§4項目3)。
    owned_land_state_count: HashMap<CountryId, u32>,
    /// 国ごとの、隣接する他国所有陸州の候補一覧(重複排除済み)。
    candidate_targets: HashMap<CountryId, HashSet<StateId>>,
}

/// StateRegistryを1回だけ走査し、`owned_land_state_count`と`candidate_targets`を
/// 同時に構築する(`countries × states`を発生させない)。同じ州ペアが隣接リストの
/// 両方向に存在していても、`candidate_targets`は`HashSet`のため重複挿入されない。
/// 1国が複数の自国州から同じ対象州に隣接していても同様に重複しない。
fn build_border_data(state_registry: &StateRegistry) -> BorderData {
    let mut owned_land_state_count: HashMap<CountryId, u32> = HashMap::new();
    let mut candidate_targets: HashMap<CountryId, HashSet<StateId>> = HashMap::new();

    for state in &state_registry.states {
        if state.is_sea {
            continue;
        }
        *owned_land_state_count
            .entry(state.owner_country_id)
            .or_insert(0) += 1;

        for &neighbor_id in &state.neighbors {
            let Some(neighbor) = state_registry.get(neighbor_id) else {
                continue;
            };
            if neighbor.is_sea || neighbor.owner_country_id == state.owner_country_id {
                continue;
            }
            candidate_targets
                .entry(state.owner_country_id)
                .or_default()
                .insert(neighbor.id);
        }
    }

    BorderData {
        owned_land_state_count,
        candidate_targets,
    }
}

/// AIによるClaim生成候補となる、資格条件(§5)を満たした1件の対象州。
#[derive(Debug, Clone, Copy, PartialEq)]
struct AiClaimCandidate {
    target_state: StateId,
    target: CountryId,
    opinion: f32,
    power_ratio: f32,
    population: u64,
    building_level_sum: u64,
}

/// §7: 複数候補からの選択順位。(1)opinionが低い (2)power ratioが高い (3)人口が多い
/// (4)建物レベル合計が高い (5)対象CountryId昇順 (6)対象StateId昇順。
/// 浮動小数点比較は`partial_cmp`が`None`を返してもpanicしないよう`Equal`へフォールバックする
/// (`ai_crisis_start::compare_candidates`と同じ規約)。
fn compare_claim_candidates(a: &AiClaimCandidate, b: &AiClaimCandidate) -> Ordering {
    a.opinion
        .partial_cmp(&b.opinion)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            b.power_ratio
                .partial_cmp(&a.power_ratio)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| b.population.cmp(&a.population))
        .then_with(|| b.building_level_sum.cmp(&a.building_level_sum))
        .then_with(|| a.target.0.cmp(&b.target.0))
        .then_with(|| a.target_state.0.cmp(&b.target_state.0))
}

/// §4: 国家単位のClaim生成資格判定(純粋関数)。国力評価が欠落している場合は
/// panicせず資格なし(`false`)として扱う(§4項目9)。
#[allow(clippy::too_many_arguments)]
fn country_can_generate_claim(
    country_id: CountryId,
    player_country: Option<CountryId>,
    is_active: bool,
    owned_land_state_count: &HashMap<CountryId, u32>,
    claimants_with_active_claim: &HashSet<CountryId>,
    claimants_with_unresolved_self_crisis: &HashSet<CountryId>,
    justification_initiators: &HashSet<CountryId>,
    war_participants: &HashSet<CountryId>,
    power_registry: &CountryPowerRegistry,
    latest_claim_month_index_by_claimant: &HashMap<CountryId, i32>,
    current_month_index: i32,
) -> bool {
    if Some(country_id) == player_country {
        return false;
    }
    // P21-021: 消滅済み(Eliminated)国家はClaim生成の対象外
    // (`owned_land_state_count`が0になり構造的にも除外されるが、is_active()を
    // 共有APIとして明示的にも確認する——§12の「~7箇所のAI列挙点でis_active()を
    // 共有する」要件に合わせる)。
    if !is_active {
        return false;
    }
    if owned_land_state_count
        .get(&country_id)
        .copied()
        .unwrap_or(0)
        == 0
    {
        return false;
    }
    if claimants_with_active_claim.contains(&country_id) {
        return false;
    }
    if claimants_with_unresolved_self_crisis.contains(&country_id) {
        return false;
    }
    if justification_initiators.contains(&country_id) {
        return false;
    }
    if war_participants.contains(&country_id) {
        return false;
    }
    if power_registry.get(country_id).is_none() {
        return false;
    }

    let latest = latest_claim_month_index_by_claimant
        .get(&country_id)
        .copied();
    claim_generation_cooldown_satisfied(latest, current_month_index)
}

/// §10: `countries × states`/`countries × claims`/`states × claims`の総当たりを避けるため、
/// Country/Claim/Crisis/正当化/War/State(境界候補)を1回ずつ走査して集合・候補一覧を
/// 事前構築してから、資格を満たした国だけを対象に候補州を評価する。
/// 国家ペアごとの資格判定(同盟・戦争・条約・国力・関係値、`evaluate_pair_eligibility`)は
/// `(claimant, target)`単位でメモ化し、同じ対象国へ複数の隣接候補州を持つclaimantが
/// いても再計算しない。
///
/// 戻り値はclaimantごとにグループ化され、各Vecは`compare_claim_candidates`で
/// 既にソート済み(先頭が最優先候補)。
#[allow(clippy::too_many_arguments)]
fn collect_ai_claim_candidates(
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
    current_month_index: i32,
) -> HashMap<CountryId, Vec<AiClaimCandidate>> {
    let claimants_with_unresolved_self_crisis = crisis_registry.unresolved_crisis_initiators();
    let justification_initiators = collect_justification_initiators(justification_registry);
    let war_participants = collect_war_participants(war_registry);
    let (claimants_with_active_claim, latest_claim_month_index_by_claimant) =
        collect_claim_history(claim_registry, current_month_index);
    // レビュー指摘対応: `ai_crisis_start::build_at_war_pairs`はWarごとに
    // `attackers.len() × defenders.len()`組を`HashSet`へ展開するため、多国間War
    // (P21-016)では`O(wars)`ではなく`O(Σ attackers×defenders)`になる
    // (1000 War×攻撃側/防御側各100か国なら10,000,000組)。P21-017はこの実コストを
    // 払う価値がある(claimantが第三国と交戦中でも、targetとは非交戦なら候補になり
    // 得るため、ペア単位の判定が必要)。しかしP21-018はclaimant自身を
    // `war_participants`(このすぐ下、O(Σ participants)の線形集合)で「現在
    // いずれのWarにも参加していない」国だけに絞り込み済みであり、
    // `evaluate_pair_eligibility`に渡す`at_war_pairs`はclaimantを含む組を一切
    // 生成できない(claimantがどのWarのattackers/defendersにも登場しないため)。
    // よってこの空集合は実際のペア集合と常に同じ判定結果を返す
    // (`.contains(&normalize_pair(claimant, target))`は常にfalse)ため、
    // ペア展開そのものを行わずに済ませる。
    let at_war_pairs: HashSet<(CountryId, CountryId)> = HashSet::new();
    let border = build_border_data(state_registry);

    let country_by_id: HashMap<CountryId, &CountryData> = country_registry
        .countries
        .iter()
        .map(|c| (c.id, c))
        .collect();

    let mut pair_cache: HashMap<(CountryId, CountryId), Option<(f32, f32)>> = HashMap::new();
    let mut grouped: HashMap<CountryId, Vec<AiClaimCandidate>> = HashMap::new();

    for country in &country_registry.countries {
        let claimant = country.id;

        if !country_can_generate_claim(
            claimant,
            player_country,
            country.is_active(),
            &border.owned_land_state_count,
            &claimants_with_active_claim,
            &claimants_with_unresolved_self_crisis,
            &justification_initiators,
            &war_participants,
            power_registry,
            &latest_claim_month_index_by_claimant,
            current_month_index,
        ) {
            continue;
        }

        let Some(targets) = border.candidate_targets.get(&claimant) else {
            continue;
        };

        for &target_state_id in targets {
            let Some(state) = state_registry.get(target_state_id) else {
                continue;
            };
            let target = state.owner_country_id;
            if !country_by_id.contains_key(&target) {
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

            let building_level_sum: u64 = state.buildings.values().map(|&lvl| lvl as u64).sum();

            grouped.entry(claimant).or_default().push(AiClaimCandidate {
                target_state: target_state_id,
                target,
                opinion,
                power_ratio,
                population: state.population,
                building_level_sum,
            });
        }
    }

    for candidates in grouped.values_mut() {
        candidates.sort_by(compare_claim_candidates);
    }

    grouped
}

/// 候補一覧(既にソート済み)を先頭から順に試し、既存の共有ドメインAPI
/// (`ClaimRegistry::create_claim`、内部で`can_create_claim`による直前の再検証を行う)に
/// 最初に成功したものを生成する。全候補が失敗した場合は何も生成しない(`None`)。
fn try_create_claim_from_candidates(
    claim_registry: &mut ClaimRegistry,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    candidates: &[AiClaimCandidate],
    claimant: CountryId,
    current_date: &str,
) -> Option<ClaimId> {
    for candidate in candidates {
        if let Ok(id) = claim_registry.create_claim(
            claimant,
            candidate.target,
            candidate.target_state,
            current_date.to_string(),
            ClaimSource::AiExpansion,
            country_registry,
            state_registry,
        ) {
            return Some(id);
        }
    }
    None
}

/// P21-018のエントリーポイント(Bevy非依存)。AI国家をCountryId昇順で処理し、
/// 各国最大1件までClaimを生成する。生成したClaimはこの呼び出し内ではCrisis化しない
/// (§11: P21-017の次回月次評価が通常のClaimとして拾う)。
#[allow(clippy::too_many_arguments)]
pub fn evaluate_ai_claim_generation(
    claim_registry: &mut ClaimRegistry,
    crisis_registry: &CrisisRegistry,
    justification_registry: &WarJustificationRegistry,
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    diplomacy_registry: &DiplomacyRegistry,
    war_registry: &WarRegistry,
    power_registry: &CountryPowerRegistry,
    player_country: &PlayerCountry,
    current_date: &str,
    current_year: i32,
    current_month: u8,
) {
    let current_month_index = month_index(current_year, current_month);

    let grouped = collect_ai_claim_candidates(
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
        current_month_index,
    );

    let mut claimants: Vec<CountryId> = grouped.keys().copied().collect();
    claimants.sort_by_key(|c| c.0);

    for claimant in claimants {
        let candidates = &grouped[&claimant];
        try_create_claim_from_candidates(
            claim_registry,
            country_registry,
            state_registry,
            candidates,
            claimant,
            current_date,
        );
    }
}

/// §9: `evaluate_ai_claim_generation`が実際に何回呼ばれたかを直接検証するための
/// 診断専用Resource。[`crate::diplomacy::ai_crisis_start::AiCrisisStartEvaluationCount`]
/// と全く同じ設計: `Option<ResMut<_>>`として受け取るため、Worldに存在しなければ
/// 静かに`None`となり本番では一切のオーバーヘッドを生まない。
#[derive(Resource, Default, Debug)]
pub struct AiClaimGenerationEvaluationCount(pub usize);

/// System: `MonthChangedMessage`が発生したフレームにつき最大1回だけ
/// `evaluate_ai_claim_generation`を実行する。`DiplomacyPlugin`が
/// `DailySimulationSet::CountryAi`内で、P21-017の月次Crisis開始・通常戦争準備AI・
/// AI宣戦布告(`handle_daily_country_ai`)より後に実行されるよう明示的に順序付ける
/// (§9: 「同じフレームで通常正当化と新規Claimを並行開始しない」「AIがWarへ入った場合、
/// そのフレームではClaimを生成しない」を、System登録順ではなく明示的な`.after()`で保証する)。
#[allow(clippy::too_many_arguments)]
pub fn handle_monthly_ai_claim_generation(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut claim_registry: ResMut<ClaimRegistry>,
    crisis_registry: Res<CrisisRegistry>,
    justification_registry: Res<WarJustificationRegistry>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    diplomacy_registry: Res<DiplomacyRegistry>,
    war_registry: Res<WarRegistry>,
    power_registry: Res<CountryPowerRegistry>,
    player_country: Res<PlayerCountry>,
    date: Res<GameDate>,
    mut eval_counter: Option<ResMut<AiClaimGenerationEvaluationCount>>,
) {
    let mut evaluated_this_frame = false;
    for _event in month_events.read() {
        if evaluated_this_frame {
            continue;
        }
        evaluated_this_frame = true;

        evaluate_ai_claim_generation(
            &mut claim_registry,
            &crisis_registry,
            &justification_registry,
            &country_registry,
            &state_registry,
            &diplomacy_registry,
            &war_registry,
            &power_registry,
            &player_country,
            &date.display(),
            date.year,
            date.month,
        );

        if let Some(counter) = eval_counter.as_mut() {
            counter.0 += 1;
        }
    }
}

#[cfg(test)]
mod tests;
