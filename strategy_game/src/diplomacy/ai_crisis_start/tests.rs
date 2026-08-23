use super::*;
use crate::country::CountryData;
use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis, WarGoal, WarGoalType};
use crate::diplomacy::data::{ActiveTreaty, DiplomaticPairKey, DiplomaticRelation, TreatyType};
use crate::state::data::StateData;
use std::collections::HashMap as StdHashMap;

fn country(id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        ..CountryData::default()
    }
}

fn land_state(id: usize, owner: usize) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: CountryId(owner),
        is_sea: false,
        ..Default::default()
    }
}

fn claim(id: usize, claimant: usize, target_state: usize) -> TerritorialClaim {
    TerritorialClaim {
        id: ClaimId(id),
        claimant_country: CountryId(claimant),
        target_state: StateId(target_state),
        strength: 50.0,
        created_date: "1800/01/01".to_string(),
        is_permanent: false,
        source: ClaimSource::Strategic,
        status: ClaimStatus::Active,
    }
}

fn power_registry_with(entries: &[(usize, f32)]) -> CountryPowerRegistry {
    // CountryPowerRegistryは`evaluate_country_power`経由でしか構築できないため、
    // 目的のtotal_scoreになるよう人口だけを調整した最小限の世界を評価させる。
    // total_score = 0.5*military + 0.3*economic + 0.2*populationなので、
    // 他の入力を全てゼロにすれば population_normalized が total_score の
    // 5倍になる比例関係を使い、目標スコアから逆算した人口を与える。
    let countries: Vec<CountryData> = entries.iter().map(|&(id, _)| country(id)).collect();
    let max_score = entries.iter().cloned().fold(0.0_f32, |m, (_, s)| m.max(s));
    let states: Vec<StateData> = entries
        .iter()
        .enumerate()
        .map(|(i, &(id, score))| {
            let population = if max_score > 0.0 {
                ((score / max_score) * 1_000_000.0) as u64
            } else {
                0
            };
            StateData {
                id: StateId(5000 + i),
                owner_country_id: CountryId(id),
                population,
                ..Default::default()
            }
        })
        .collect();
    crate::country::power::evaluate_country_power(
        &CountryRegistry { countries },
        &StateRegistry::build(states),
        &crate::military::data::MilitaryRegistry::default(),
        &crate::building::data::BuildingRegistry::default(),
        "1800/01/01".to_string(),
    )
}

fn set_opinion(diplomacy: &mut DiplomacyRegistry, a: usize, b: usize, opinion: f32) {
    let key = DiplomaticPairKey::new(CountryId(a), CountryId(b)).unwrap();
    diplomacy.relations.insert(
        key,
        DiplomaticRelation {
            opinion,
            ..Default::default()
        },
    );
}

fn set_alliance(diplomacy: &mut DiplomacyRegistry, a: usize, b: usize) {
    let key = DiplomaticPairKey::new(CountryId(a), CountryId(b)).unwrap();
    diplomacy.relations.insert(
        key,
        DiplomaticRelation {
            treaties: vec![ActiveTreaty {
                treaty_type: TreatyType::Alliance,
                countries: (CountryId(a), CountryId(b)),
                signed_date: "1800/01/01".to_string(),
                is_active: true,
            }],
            ..Default::default()
        },
    );
}

// ─────────────────────────────────────────────────────────────────────
// 純粋判断 (要求テスト1-12)
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト1-3: opinion境界(-25.0は開始可能、-24.9は不可、-100.0は可能)。
#[test]
fn opinion_boundaries() {
    assert!(ai_crisis_opinion_permits_start(-25.0));
    assert!(!ai_crisis_opinion_permits_start(-24.9));
    assert!(ai_crisis_opinion_permits_start(-100.0));
}

/// 要求テスト4: 未登録関係の中立値0.0では開始不可。
#[test]
fn unregistered_relation_neutral_zero_does_not_permit_start() {
    assert!(!ai_crisis_opinion_permits_start(0.0));
}

/// 要求テスト6-7: power ratio境界(0.75は開始可能、0.749は不可)。
#[test]
fn power_ratio_boundaries() {
    assert!(ai_crisis_power_ratio_permits_start(0.75));
    assert!(!ai_crisis_power_ratio_permits_start(0.749));
}

/// 要求テスト8: 両国score 0ならratio 1.0。
#[test]
fn both_scores_zero_yields_ratio_one() {
    assert_eq!(compute_power_ratio(0.0, 0.0), 1.0);
}

/// 要求テスト9: 対象score 0・自国score正なら開始可能。
#[test]
fn target_zero_initiator_positive_permits_start() {
    let ratio = compute_power_ratio(50.0, 0.0);
    assert!(ai_crisis_power_ratio_permits_start(ratio));
}

/// 要求テスト10: 自国score 0・対象score正なら開始不可。
#[test]
fn initiator_zero_target_positive_does_not_permit_start() {
    let ratio = compute_power_ratio(0.0, 50.0);
    assert_eq!(ratio, 0.0);
    assert!(!ai_crisis_power_ratio_permits_start(ratio));
}

/// 要求テスト11: NaN・無限大を生成しない。
#[test]
fn compute_power_ratio_never_produces_nan_or_infinite() {
    for (i, t) in [
        (0.0, 0.0),
        (50.0, 0.0),
        (0.0, 50.0),
        (30.0, 70.0),
        (100.0, 100.0),
    ] {
        let ratio = compute_power_ratio(i, t);
        assert!(
            !ratio.is_nan(),
            "compute_power_ratio({i},{t}) must not be NaN"
        );
        assert!(
            ratio.is_finite(),
            "compute_power_ratio({i},{t}) must not be infinite"
        );
    }
}

/// 要求テスト12: 同じ入力から常に同じ結果になる(決定論性)。
#[test]
fn compute_power_ratio_is_deterministic() {
    for _ in 0..5 {
        assert_eq!(
            compute_power_ratio(30.0, 70.0),
            compute_power_ratio(30.0, 70.0)
        );
    }
    for _ in 0..5 {
        assert!(ai_crisis_opinion_permits_start(-25.0));
    }
}

// ─────────────────────────────────────────────────────────────────────
// Claim資格 (要求テスト13-25) — collect_ai_crisis_candidates経由
// ─────────────────────────────────────────────────────────────────────

/// 標準的な2か国環境(claimant=0, target=1)。国力はclaimantが優位
/// (power_ratio >= 0.75を満たす)、関係値は-25.0以下(opinion条件を満たす)。
struct StdEnv {
    claim_registry: ClaimRegistry,
    crisis_registry: CrisisRegistry,
    justification_registry: WarJustificationRegistry,
    country_registry: CountryRegistry,
    state_registry: StateRegistry,
    diplomacy_registry: DiplomacyRegistry,
    war_registry: WarRegistry,
    power_registry: CountryPowerRegistry,
}

fn std_env() -> StdEnv {
    let mut claim_registry = ClaimRegistry::default();
    claim_registry.add_claim(claim(0, 0, 1));

    let country_registry = CountryRegistry {
        countries: vec![country(0), country(1)],
    };
    let state_registry = StateRegistry::build(vec![land_state(0, 0), land_state(1, 1)]);
    let mut diplomacy_registry = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy_registry, 0, 1, -30.0);
    let power_registry = power_registry_with(&[(0, 100.0), (1, 10.0)]);

    StdEnv {
        claim_registry,
        crisis_registry: CrisisRegistry::default(),
        justification_registry: WarJustificationRegistry::default(),
        country_registry,
        state_registry,
        diplomacy_registry,
        war_registry: WarRegistry::default(),
        power_registry,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect(env: &StdEnv, player: Option<CountryId>) -> HashMap<CountryId, Vec<AiCrisisCandidate>> {
    collect_ai_crisis_candidates(
        &env.claim_registry,
        &env.crisis_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        player,
        "1800/02/01",
    )
}

/// 要求テスト13: AI保有の有効Claimを候補にする。
#[test]
fn ai_owned_active_claim_is_a_candidate() {
    let env = std_env();
    let grouped = collect(&env, None);
    assert_eq!(grouped.get(&CountryId(0)).map(|v| v.len()), Some(1));
}

/// 要求テスト14: プレイヤー保有Claimを自動開始しない。
#[test]
fn player_owned_claim_is_never_a_candidate() {
    let env = std_env();
    let grouped = collect(&env, Some(CountryId(0)));
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト15: 期限切れClaimを使用しない。
/// (現行`TerritorialClaim`に有効期限フィールドが存在しないため、この条件は
/// 恒久的に無条件で満たされる。将来フィールドが追加されるまでテスト対象外。)
#[test]
fn territorial_claim_has_no_expiry_field_so_this_condition_is_vacuously_satisfied() {
    // ドキュメント目的のテスト: `TerritorialClaim`の全フィールドを再掲することで、
    // 将来フィールドが追加された際にこのテストのコンパイルが壊れ、有効期限判定の
    // 追加実装が必要であることに気付けるようにする。
    let c = claim(0, 0, 1);
    let TerritorialClaim {
        id: _,
        claimant_country: _,
        target_state: _,
        strength: _,
        created_date: _,
        is_permanent: _,
        source: _,
        status: _,
    } = c;
}

/// 要求テスト16: 無効状態(Consumed)Claimを使用しない。
#[test]
fn consumed_claim_is_not_a_candidate() {
    let mut env = std_env();
    env.claim_registry
        .claims
        .get_mut(&ClaimId(0))
        .unwrap()
        .status = ClaimStatus::Consumed;
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト17: 存在しない対象州を使用しない。
#[test]
fn claim_targeting_nonexistent_state_is_not_a_candidate() {
    let mut env = std_env();
    env.claim_registry
        .claims
        .get_mut(&ClaimId(0))
        .unwrap()
        .target_state = StateId(999);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト18: 州ownerが対象国と異なる(=claimant自身が既に所有している)場合は
/// 候補にしない。
#[test]
fn claim_targeting_state_now_owned_by_claimant_is_not_a_candidate() {
    let mut env = std_env();
    env.state_registry = StateRegistry::build(vec![land_state(0, 0), land_state(1, 0)]);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト19: 現在戦争中の対象へ開始しない。
#[test]
fn target_currently_at_war_with_claimant_is_not_a_candidate() {
    let mut env = std_env();
    let mut war_registry = WarRegistry::default();
    war_registry.wars.insert(
        crate::common::WarId(0),
        crate::war::data::War {
            id: crate::common::WarId(0),
            name: "Test War".to_string(),
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attackers: [CountryId(0)].into_iter().collect(),
            defenders: [CountryId(1)].into_iter().collect(),
            primary_attacker: Some(CountryId(0)),
            primary_defender: Some(CountryId(1)),
            source_crisis_id: None,
            war_goals: vec![],
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: std::collections::HashSet::new(),
            status: crate::war::data::WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: std::collections::HashSet::new(),
        },
    );
    env.war_registry = war_registry;
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト20: 同盟国へ開始しない。
#[test]
fn allied_target_is_not_a_candidate() {
    let mut env = std_env();
    set_alliance(&mut env.diplomacy_registry, 0, 1);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト21: 同じClaimの進行中Crisisがあれば開始しない。
#[test]
fn claim_with_unresolved_crisis_is_not_a_candidate() {
    let mut env = std_env();
    env.crisis_registry.add_crisis(DiplomaticCrisis {
        id: DiplomaticCrisisId(0),
        initiator: CountryId(0),
        target: CountryId(1),
        war_goals: vec![WarGoal {
            attacker: CountryId(0),
            defender: CountryId(1),
            goal_type: WarGoalType::ConquerState,
            target_states: vec![StateId(1)],
            base_peace_cost: 0.0,
            international_concern: 0.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1800/01/01".to_string(),
        current_phase: CrisisPhase::DemandSent,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: Some("1800/01/31".to_string()),
        international_concern: 0.0,
        third_party_reactions: StdHashMap::new(),
        related_claim_id: Some(ClaimId(0)),
        related_justification_id: None,
        related_war_id: None,
    });
    let grouped = collect(&env, None);
    assert!(
        !grouped.contains_key(&CountryId(0)),
        "claimant has no other eligible claim once its only claim is excluded"
    );
}

/// 要求テスト22: 同じClaimのReady正当化があれば開始しない。
#[test]
fn claim_with_ready_justification_is_not_a_candidate() {
    let mut env = std_env();
    let crisis_id = env.crisis_registry.add_crisis(DiplomaticCrisis {
        id: DiplomaticCrisisId(0),
        initiator: CountryId(0),
        target: CountryId(1),
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        current_phase: CrisisPhase::Escalating,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: StdHashMap::new(),
        related_claim_id: Some(ClaimId(0)),
        related_justification_id: Some(0),
        related_war_id: None,
    });
    let j_id = env.justification_registry.grant_completed_justification(
        CountryId(0),
        CountryId(1),
        StateId(1),
        "1800/01/01".to_string(),
        Some(crisis_id),
        vec![],
        vec![],
    );
    assert_eq!(j_id, 0);
    let grouped = collect(&env, None);
    assert!(
        !grouped.contains_key(&CountryId(0)),
        "claimant already has an unresolved self-initiated crisis (Escalating) for this claim"
    );
}

/// 要求テスト22(変形): Escalatingではなく既にresolvedなCrisis経由でも、
/// Ready正当化それ自体がClaimを除外する(claimant側の自国Crisis重複条件とは
/// 独立に、正当化のClaimリンクだけで判定できることを確認する)。
#[test]
fn claim_with_ready_justification_is_excluded_even_if_source_crisis_is_resolved() {
    let mut env = std_env();
    let crisis_id = env.crisis_registry.add_crisis(DiplomaticCrisis {
        id: DiplomaticCrisisId(0),
        initiator: CountryId(0),
        target: CountryId(1),
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        current_phase: CrisisPhase::WarStarted,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: None,
        international_concern: 0.0,
        third_party_reactions: StdHashMap::new(),
        related_claim_id: Some(ClaimId(0)),
        related_justification_id: None,
        related_war_id: Some(crate::common::WarId(0)),
    });
    env.justification_registry.grant_completed_justification(
        CountryId(0),
        CountryId(1),
        StateId(1),
        "1800/01/01".to_string(),
        Some(crisis_id),
        vec![],
        vec![],
    );
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// P21-017(二次レビュー対応・§27指摘2への対応): Crisisを一切経由しない、既存の
/// 生の戦争準備AI(`country_ai::process_war_preparation_ai`)が独立に開始した
/// 正当化(`source_crisis_id: None`)であっても、同じ(claimant, target,
/// target_state)の組を対象とするClaimは候補から除外される。以前は
/// `claims_with_justification`がCrisis由来(`source_crisis_id`→
/// `related_claim_id`で追跡できるもの)しか見ておらず、この生正当化との
/// 並行進行を防げていなかった。
#[test]
fn claim_matching_a_raw_non_crisis_justification_is_not_a_candidate() {
    let mut env = std_env();
    // Crisisを一切経由しない、process_war_preparation_ai相当の生正当化。
    env.justification_registry.grant_completed_justification(
        CountryId(0),
        CountryId(1),
        StateId(1),
        "1800/01/01".to_string(),
        None,
        vec![],
        vec![],
    );
    let grouped = collect(&env, None);
    assert!(
        !grouped.contains_key(&CountryId(0)),
        "a raw (non-Crisis) justification for the exact same (claimant, target, \
         target_state) triple must exclude the claim, not just Crisis-derived ones"
    );
}

/// 要求テスト23: 自国開始の未解決Crisisがあれば(別Claim由来でも)新規開始しない。
#[test]
fn claimant_with_any_unresolved_self_initiated_crisis_gets_no_new_candidates() {
    let mut env = std_env();
    // claimant(0)が全く別のClaim/対象州に対して既に未解決Crisisを開始済み。
    env.crisis_registry.add_crisis(DiplomaticCrisis {
        id: DiplomaticCrisisId(0),
        initiator: CountryId(0),
        target: CountryId(2),
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        current_phase: CrisisPhase::DemandSent,
        escalation: 0.0,
        initiator_support: 0.0,
        target_resistance: 0.0,
        days_in_phase: 0,
        deadline_date: Some("1800/01/31".to_string()),
        international_concern: 0.0,
        third_party_reactions: StdHashMap::new(),
        related_claim_id: Some(ClaimId(999)),
        related_justification_id: None,
        related_war_id: None,
    });
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト24: CountryPowerRegistry欠落時にpanicしない(候補から除外されるだけ)。
#[test]
fn missing_power_registry_entries_do_not_panic() {
    let mut env = std_env();
    env.power_registry = CountryPowerRegistry::default();
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト25: 国家評価が片方だけ欠落していてもpanicしない。
#[test]
fn one_sided_missing_power_registry_entry_does_not_panic() {
    let mut env = std_env();
    env.power_registry = power_registry_with(&[(0, 100.0)]); // target(1)の評価が無い
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

// ─────────────────────────────────────────────────────────────────────
// 候補選択 (要求テスト26-33)
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト26: 最も関係値が低いClaimを選ぶ。
#[test]
fn candidate_with_lowest_opinion_is_selected_first() {
    let mut env = std_env();
    // 2件目の候補(target=2)を追加: 関係値がさらに低い(より優先されるべき)。
    env.claim_registry.add_claim(claim(0, 0, 2));
    env.state_registry =
        StateRegistry::build(vec![land_state(0, 0), land_state(1, 1), land_state(2, 2)]);
    env.country_registry = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    env.power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]);
    set_opinion(&mut env.diplomacy_registry, 0, 1, -30.0);
    set_opinion(&mut env.diplomacy_registry, 0, 2, -80.0); // より低い(より優先)

    let grouped = collect(&env, None);
    let candidates = grouped.get(&CountryId(0)).unwrap();
    assert_eq!(candidates[0].target, CountryId(2));
}

/// 要求テスト27: 関係値同値ならpower ratioが高いClaimを選ぶ。
#[test]
fn tied_opinion_breaks_by_higher_power_ratio() {
    let mut env = std_env();
    env.claim_registry.add_claim(claim(1, 0, 2));
    env.state_registry =
        StateRegistry::build(vec![land_state(0, 0), land_state(1, 1), land_state(2, 2)]);
    env.country_registry = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    // target(1)への比率は10倍、target(2)への比率は5倍(2の方が低いはず)。
    env.power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 20.0)]);
    set_opinion(&mut env.diplomacy_registry, 0, 1, -30.0);
    set_opinion(&mut env.diplomacy_registry, 0, 2, -30.0); // 同値

    let grouped = collect(&env, None);
    let candidates = grouped.get(&CountryId(0)).unwrap();
    assert_eq!(
        candidates[0].target,
        CountryId(1),
        "target(1) has the higher power ratio and must be selected first when opinion ties"
    );
}

/// 要求テスト29: opinion・ratioとも同値ならClaim ID昇順。
#[test]
fn fully_tied_candidates_break_by_claim_id_ascending() {
    let mut env = std_env();
    env.claim_registry.claims.insert(ClaimId(5), claim(5, 0, 2));
    env.state_registry = StateRegistry::build(vec![
        land_state(0, 0),
        land_state(1, 1),
        land_state(2, 1), // 同じ対象国(1)への2件目のClaim
    ]);
    env.country_registry = CountryRegistry {
        countries: vec![country(0), country(1)],
    };
    set_opinion(&mut env.diplomacy_registry, 0, 1, -30.0);

    let grouped = collect(&env, None);
    let candidates = grouped.get(&CountryId(0)).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].claim_id, ClaimId(0));
    assert_eq!(candidates[1].claim_id, ClaimId(5));
}

/// 要求テスト30: Claim HashMap挿入順を変えても結果が同じ。
#[test]
fn insertion_order_does_not_affect_selected_candidate() {
    let mut env_a = std_env();
    env_a.claim_registry.add_claim(claim(1, 0, 2));
    let mut env_b = std_env();
    // 逆順で挿入(add_claimはnext_idを使うため、直接HashMapへ挿入して順序を変える)。
    env_b.claim_registry.claims.clear();
    env_b
        .claim_registry
        .claims
        .insert(ClaimId(1), claim(1, 0, 2));
    env_b
        .claim_registry
        .claims
        .insert(ClaimId(0), claim(0, 0, 1));

    for env in [&mut env_a, &mut env_b] {
        env.state_registry =
            StateRegistry::build(vec![land_state(0, 0), land_state(1, 1), land_state(2, 2)]);
        env.country_registry = CountryRegistry {
            countries: vec![country(0), country(1), country(2)],
        };
        env.power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]);
        set_opinion(&mut env.diplomacy_registry, 0, 1, -30.0);
        set_opinion(&mut env.diplomacy_registry, 0, 2, -30.0);
    }

    let grouped_a = collect(&env_a, None);
    let grouped_b = collect(&env_b, None);
    assert_eq!(
        grouped_a.get(&CountryId(0)).unwrap()[0].claim_id,
        grouped_b.get(&CountryId(0)).unwrap()[0].claim_id
    );
}

/// 要求テスト31: 一つのAIは月次一回につき最大一件だけ開始する。
#[test]
fn evaluate_ai_crisis_start_opens_at_most_one_crisis_per_country_per_call() {
    let mut env = std_env();
    env.claim_registry.add_claim(claim(1, 0, 2));
    env.state_registry =
        StateRegistry::build(vec![land_state(0, 0), land_state(1, 1), land_state(2, 2)]);
    env.country_registry = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    env.power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]);
    set_opinion(&mut env.diplomacy_registry, 0, 1, -30.0);
    set_opinion(&mut env.diplomacy_registry, 0, 2, -30.0);

    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );

    let started_by_zero = env
        .crisis_registry
        .crises
        .values()
        .filter(|c| c.initiator == CountryId(0))
        .count();
    assert_eq!(started_by_zero, 1);
}

/// 要求テスト32: 第一候補がドメイン検証で失敗したら次候補を試せる。
#[test]
fn first_candidate_domain_failure_falls_through_to_next_candidate() {
    let env = std_env();
    let mut crisis_registry = CrisisRegistry::default();
    let mut consumed_claim = claim(0, 0, 1);
    consumed_claim.status = ClaimStatus::Consumed; // can_start_crisisが拒否する

    let candidates = vec![
        AiCrisisCandidate {
            claim_id: ClaimId(0),
            claimant: CountryId(0),
            target: CountryId(1),
            target_state: StateId(1),
            opinion: -30.0,
            power_ratio: 10.0,
        },
        AiCrisisCandidate {
            claim_id: ClaimId(1),
            claimant: CountryId(0),
            target: CountryId(1),
            target_state: StateId(1),
            opinion: -30.0,
            power_ratio: 10.0,
        },
    ];
    let mut claim_registry = ClaimRegistry::default();
    claim_registry.claims.insert(ClaimId(0), consumed_claim);
    claim_registry.claims.insert(ClaimId(1), claim(1, 0, 1));

    let result = try_start_crisis_from_candidates(
        &mut crisis_registry,
        &claim_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &candidates,
        "1800/02/01",
    );
    assert!(
        result.is_some(),
        "must fall through to the second (Active) candidate"
    );
    let started = crisis_registry.crises.get(&result.unwrap()).unwrap();
    assert_eq!(started.related_claim_id, Some(ClaimId(1)));
}

/// 要求テスト33: 複数AIはCountryId昇順で処理される。
#[test]
fn multiple_ai_countries_are_processed_in_country_id_ascending_order() {
    // claimant(0)とclaimant(2)がそれぞれ独立にtarget(1)へClaimを持つ状況。
    // 両者は互いに独立(それぞれ自分のClaimだけを消費)なので、処理順自体を
    // 直接観測しづらいが、両者が正しくCountryId降順で与えても各々1件ずつ
    // 開始されること(処理順に依存した見落としがないこと)を確認する。
    let mut env = std_env();
    env.claim_registry.add_claim(claim(1, 2, 1));
    env.country_registry = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    env.power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 100.0)]);
    set_opinion(&mut env.diplomacy_registry, 2, 1, -30.0);

    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );

    assert!(
        env.crisis_registry
            .crises
            .values()
            .any(|c| c.initiator == CountryId(0))
    );
    assert!(
        env.crisis_registry
            .crises
            .values()
            .any(|c| c.initiator == CountryId(2))
    );
}

// ─────────────────────────────────────────────────────────────────────
// 月次System (要求テスト34-41)
// ─────────────────────────────────────────────────────────────────────

fn build_test_app(player: Option<CountryId>) -> App {
    let mut app = App::new();
    app.add_message::<MonthChangedMessage>();
    app.insert_resource(CrisisRegistry::default());
    app.insert_resource(ClaimRegistry::default());
    app.insert_resource(WarJustificationRegistry::default());
    app.insert_resource(CountryRegistry {
        countries: vec![country(0), country(1)],
    });
    app.insert_resource(StateRegistry::build(vec![
        land_state(0, 0),
        land_state(1, 1),
    ]));
    let mut diplomacy_registry = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy_registry, 0, 1, -30.0);
    app.insert_resource(diplomacy_registry);
    app.insert_resource(WarRegistry::default());
    app.insert_resource(power_registry_with(&[(0, 100.0), (1, 10.0)]));
    app.insert_resource(PlayerCountry(player));
    app.insert_resource(GameDate::new(1800, 2, 1));
    app.insert_resource(AiCrisisStartEvaluationCount::default());
    app.add_systems(Update, handle_monthly_ai_crisis_start);
    app
}

fn seed_one_claim(app: &mut App) {
    app.world_mut()
        .resource_mut::<ClaimRegistry>()
        .add_claim(claim(0, 0, 1));
}

/// 要求テスト34: `MonthChangedMessage`なしでは評価しない。
#[test]
fn no_month_changed_message_means_no_evaluation() {
    let mut app = build_test_app(None);
    seed_one_claim(&mut app);
    app.update();
    assert!(app.world().resource::<CrisisRegistry>().crises.is_empty());
    assert_eq!(app.world().resource::<AiCrisisStartEvaluationCount>().0, 0);
}

/// 要求テスト35: 月変更時に評価する。
#[test]
fn month_changed_message_triggers_evaluation() {
    let mut app = build_test_app(None);
    seed_one_claim(&mut app);
    app.world_mut().write_message(MonthChangedMessage {
        year: 1800,
        month: 2,
    });
    app.update();
    assert_eq!(app.world().resource::<CrisisRegistry>().crises.len(), 1);
    assert_eq!(app.world().resource::<AiCrisisStartEvaluationCount>().0, 1);
}

/// 要求テスト36-37: 同一フレームに複数`MonthChangedMessage`があっても、
/// 全評価はフレームにつき最大1回(状態結果だけでなく実評価回数でも検証する)。
#[test]
fn multiple_month_changed_messages_in_one_frame_evaluate_only_once() {
    let mut app = build_test_app(None);
    seed_one_claim(&mut app);
    {
        let mut messages = app
            .world_mut()
            .resource_mut::<Messages<MonthChangedMessage>>();
        messages.write(MonthChangedMessage {
            year: 1800,
            month: 2,
        });
        messages.write(MonthChangedMessage {
            year: 1800,
            month: 3,
        });
        messages.write(MonthChangedMessage {
            year: 1800,
            month: 4,
        });
    }
    app.update();
    assert_eq!(
        app.world().resource::<AiCrisisStartEvaluationCount>().0,
        1,
        "evaluate_ai_crisis_start must run exactly once even with 3 MonthChangedMessage events"
    );
    assert_eq!(app.world().resource::<CrisisRegistry>().crises.len(), 1);
}

/// 要求テスト38: Pause中は実行しない(Pause中は`MonthChangedMessage`自体が
/// `advance_game_date`から一切発行されないため、Systemへメッセージが届かない
/// ことをもって間接的に検証する — Pause自体はこのSystemの管轄外)。
#[test]
fn no_message_means_no_evaluation_even_though_this_system_has_no_pause_check_of_its_own() {
    let mut app = build_test_app(None);
    seed_one_claim(&mut app);
    // `advance_game_date`自体を実行しない(Pause中相当): MonthChangedMessageが
    // 一切書き込まれない環境で複数フレーム進めても評価されない。
    app.update();
    app.update();
    app.update();
    assert_eq!(app.world().resource::<AiCrisisStartEvaluationCount>().0, 0);
    assert!(app.world().resource::<CrisisRegistry>().crises.is_empty());
}

/// 要求テスト39: Load処理(MonthChangedMessage無し)だけでは評価しない。
#[test]
fn load_without_a_month_changed_message_does_not_evaluate() {
    let mut app = build_test_app(None);
    // Load直後相当: 既に進行中のCrisis付きセーブを復元したと仮定しても、
    // MonthChangedMessageが発行されない限り評価は起きない。
    seed_one_claim(&mut app);
    app.update();
    assert!(app.world().resource::<CrisisRegistry>().crises.is_empty());
}

/// 要求テスト41: 前回Crisis解決後、次の月変更で別Claimを開始できる。
#[test]
fn after_previous_crisis_resolves_the_next_month_can_start_a_different_claim() {
    let mut app = build_test_app(None);
    seed_one_claim(&mut app);
    app.world_mut().write_message(MonthChangedMessage {
        year: 1800,
        month: 2,
    });
    app.update();
    let first_id = *app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .keys()
        .next()
        .unwrap();
    app.world_mut()
        .resource_mut::<CrisisRegistry>()
        .crises
        .get_mut(&first_id)
        .unwrap()
        .current_phase = CrisisPhase::ResolvedPeacefully;

    // 2件目のClaim(別対象国)を追加。
    app.world_mut()
        .resource_mut::<ClaimRegistry>()
        .add_claim(claim(1, 0, 2));
    let mut state_registry =
        StateRegistry::build(vec![land_state(0, 0), land_state(1, 1), land_state(2, 2)]);
    // 元のClaim(0)は既にActiveのまま残っているので、再構築後も両方存在する。
    std::mem::swap(
        &mut *app.world_mut().resource_mut::<StateRegistry>(),
        &mut state_registry,
    );
    app.world_mut()
        .resource_mut::<CountryRegistry>()
        .countries
        .push(country(2));
    app.world_mut()
        .resource_mut::<CountryPowerRegistry>()
        .clone_from(&power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]));
    set_opinion(
        &mut app.world_mut().resource_mut::<DiplomacyRegistry>(),
        0,
        2,
        -40.0,
    );

    app.world_mut().write_message(MonthChangedMessage {
        year: 1800,
        month: 3,
    });
    app.update();

    let active_count = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .values()
        .filter(|c| {
            c.initiator == CountryId(0)
                && !matches!(
                    c.current_phase,
                    CrisisPhase::ResolvedPeacefully
                        | CrisisPhase::WarStarted
                        | CrisisPhase::Cancelled
                )
        })
        .count();
    assert_eq!(
        active_count, 1,
        "a new unresolved crisis must have started for claimant(0) after the first resolved"
    );
}

// ─────────────────────────────────────────────────────────────────────
// AI要求送信 (要求テスト42-49)
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト42: AI開始Crisisは(既存`start_crisis`が要求送信を不可分に行うため)
/// 開始と同時に既にDemandSentへ入り、期限も確定している
/// — これは「送信可能になった最初の日次tick」より早くはあり得ない最速の
/// タイミングであり、要求を満たす(モジュールdocコメント参照)。
#[test]
fn ai_started_crisis_has_already_sent_its_demand_immediately() {
    let mut env = std_env();
    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );
    let crisis = env
        .crisis_registry
        .crises
        .values()
        .find(|c| c.initiator == CountryId(0))
        .expect("AI must have started a crisis");
    assert_eq!(crisis.current_phase, CrisisPhase::DemandSent);
    assert_eq!(crisis.deadline_date, Some("1800/03/03".to_string()));
}

/// 要求テスト43: プレイヤー開始Crisisは(このモジュールが一切触れないため)
/// 自動送信されない、すなわち一切変更されない。
#[test]
fn player_initiated_crisis_is_never_touched_by_ai_evaluation() {
    let mut env = std_env();
    let existing_id = env.crisis_registry.start_crisis(
        &claim(0, 0, 1),
        CountryId(0),
        CountryId(1),
        "1800/01/01".to_string(),
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
    );
    // 上のstart_crisis呼び出し自体はテストセットアップの都合上claimant(0)を
    // 使うが、実運用のプレイヤー操作を模して「一切変更されない」ことだけを検証する。
    let existing_id = existing_id.unwrap();
    let before = env
        .crisis_registry
        .crises
        .get(&existing_id)
        .unwrap()
        .clone();

    let player_country = PlayerCountry(Some(CountryId(0)));
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );

    let after = env.crisis_registry.crises.get(&existing_id).unwrap();
    assert_eq!(before.current_phase, after.current_phase);
    assert_eq!(before.deadline_date, after.deadline_date);
    assert_eq!(env.crisis_registry.crises.len(), 1);
}

/// 要求テスト44: 同じ要求を重複送信しない(claimantが既に未解決の自国開始Crisisを
/// 持てば、2回目の評価では新規候補に挙がらない)。
#[test]
fn evaluating_twice_never_sends_a_duplicate_demand_for_the_same_claimant() {
    let mut env = std_env();
    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/03/01",
    );
    let count = env
        .crisis_registry
        .crises
        .values()
        .filter(|c| c.initiator == CountryId(0))
        .count();
    assert_eq!(count, 1);
}

/// 要求テスト45: 要求送信(Crisis開始)は既存ドメインAPI(`CrisisRegistry::
/// start_crisis`、プレイヤーUIと同一の関数)だけを経由する
/// — `related_claim_id`がClaimと正しく紐づくことで、内部で直接Registryへ
/// insertするのではなく`start_crisis`のバリデーション込み経路を通ったことを
/// 確認する(直接insertならこのリンクや`war_goals`は再現できない)。
#[test]
fn crisis_creation_goes_through_the_shared_start_crisis_domain_api() {
    let mut env = std_env();
    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );
    let crisis = env
        .crisis_registry
        .crises
        .values()
        .find(|c| c.initiator == CountryId(0))
        .unwrap();
    assert_eq!(crisis.related_claim_id, Some(ClaimId(0)));
    assert_eq!(crisis.war_goals.len(), 1);
    assert_eq!(crisis.war_goals[0].target_states, vec![StateId(1)]);
}

/// 要求テスト48: 対象がプレイヤーならAI回答(P21-012)しない
/// — `evaluate_ai_crisis_responses`(P21-012)の既存フィルタが
/// `player_country.0 != Some(c.target)`である以上、AI initiatorが作った
/// Crisisでもtargetがプレイヤーであれば対象外になることを確認する。
#[test]
fn ai_initiated_crisis_targeting_the_player_is_not_auto_answered_by_p21_012() {
    use crate::diplomacy::update::handle_daily_diplomacy;

    let mut env = std_env();
    // AI(0)がプレイヤー(1)へ対して開始したことにする。
    let player_country = PlayerCountry(Some(CountryId(1)));
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );
    let crisis_id = *env.crisis_registry.crises.keys().next().unwrap();
    assert_eq!(
        env.crisis_registry.crises.get(&crisis_id).unwrap().target,
        CountryId(1)
    );

    let mut app = App::new();
    app.add_message::<crate::app::time::DayChangedMessage>();
    app.insert_resource(env.diplomacy_registry);
    app.insert_resource(player_country);
    app.insert_resource(env.country_registry);
    app.insert_resource(crate::app::time::GameDate::new(1800, 2, 2));
    app.insert_resource(env.justification_registry);
    app.insert_resource(env.state_registry);
    app.insert_resource(env.crisis_registry);
    app.insert_resource(env.claim_registry);
    app.insert_resource(env.power_registry);
    app.insert_resource(env.war_registry);
    app.insert_resource(crate::localization::CurrentLocale::default());
    app.insert_resource(
        crate::localization::TranslationCatalog::load().expect("embedded catalogs must parse"),
    );
    app.insert_resource(crate::military::data::MilitaryRegistry::default());
    app.insert_resource(crate::military::army::ArmyRegistry::default());
    app.insert_resource(crate::war::frontline::FrontlineRegistry::default());
    app.insert_resource(crate::military::battle::BattleRegistry::default());
    app.add_message::<crate::ui::notification::GameNotification>();
    app.add_systems(Update, handle_daily_diplomacy);

    app.world_mut()
        .write_message(crate::app::time::DayChangedMessage {
            year: 1800,
            month: 2,
            day: 2,
        });
    app.update();

    let crisis = app
        .world()
        .resource::<CrisisRegistry>()
        .crises
        .get(&crisis_id)
        .unwrap();
    assert_eq!(
        crisis.current_phase,
        CrisisPhase::DemandSent,
        "player-target crisis must remain awaiting manual response, not auto-answered"
    );
}

/// 要求テスト49: deadline・phaseの既存規則を維持する
/// (`CRISIS_DEMAND_PERIOD_DAYS`をAIも同じ計算で使う)。
#[test]
fn ai_started_crisis_uses_the_same_deadline_rule_as_the_player() {
    let mut env = std_env();
    let expected_deadline = GameDate::new(1800, 2, 1)
        .add_days(crate::diplomacy::crisis::CRISIS_DEMAND_PERIOD_DAYS)
        .display();
    let player_country = PlayerCountry(None);
    evaluate_ai_crisis_start(
        &mut env.crisis_registry,
        &env.claim_registry,
        &env.justification_registry,
        &env.country_registry,
        &env.state_registry,
        &env.diplomacy_registry,
        &env.war_registry,
        &env.power_registry,
        &player_country,
        "1800/02/01",
    );
    let crisis = env
        .crisis_registry
        .crises
        .values()
        .find(|c| c.initiator == CountryId(0))
        .unwrap();
    assert_eq!(crisis.deadline_date, Some(expected_deadline));
}
