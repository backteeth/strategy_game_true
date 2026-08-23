use super::*;
use crate::country::CountryData;
use crate::country::country_ai::{CountryAiRegistry, process_daily_country_ai};
use crate::diplomacy::claims::TerritorialClaim;
use crate::diplomacy::crisis::{CrisisPhase, DiplomaticCrisis};
use crate::diplomacy::data::{ActiveTreaty, DiplomaticPairKey, DiplomaticRelation, TreatyType};
use crate::state::data::StateData;
use crate::war::data::War;
use crate::war::justification::WarJustification;
use std::collections::HashMap as StdHashMap;

fn country(id: usize) -> CountryData {
    CountryData {
        id: CountryId(id),
        ..CountryData::default()
    }
}

fn land_state(id: usize, owner: usize, neighbors: &[usize]) -> StateData {
    StateData {
        id: StateId(id),
        owner_country_id: CountryId(owner),
        is_sea: false,
        neighbors: neighbors.iter().map(|&n| StateId(n)).collect(),
        ..Default::default()
    }
}

fn sea_state(id: usize, owner: usize, neighbors: &[usize]) -> StateData {
    let mut s = land_state(id, owner, neighbors);
    s.is_sea = true;
    s
}

fn claim(id: usize, claimant: usize, target_state: usize, created_date: &str) -> TerritorialClaim {
    TerritorialClaim {
        id: ClaimId(id),
        claimant_country: CountryId(claimant),
        target_state: StateId(target_state),
        strength: 50.0,
        created_date: created_date.to_string(),
        is_permanent: false,
        source: ClaimSource::Strategic,
        status: ClaimStatus::Active,
    }
}

fn power_registry_with(entries: &[(usize, f32)]) -> CountryPowerRegistry {
    // CountryPowerRegistryは`evaluate_country_power`経由でしか構築できないため、
    // 目的のtotal_scoreになるよう人口だけを調整した最小限の世界を評価させる
    // (`ai_crisis_start::tests::power_registry_with`と同じ手法)。
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

fn set_treaty(diplomacy: &mut DiplomacyRegistry, a: usize, b: usize, treaty_type: TreatyType) {
    let key = DiplomaticPairKey::new(CountryId(a), CountryId(b)).unwrap();
    diplomacy.relations.insert(
        key,
        DiplomaticRelation {
            treaties: vec![ActiveTreaty {
                treaty_type,
                countries: (CountryId(a), CountryId(b)),
                signed_date: "1800/01/01".to_string(),
                is_active: true,
            }],
            ..Default::default()
        },
    );
}

fn active_war(id: usize, attackers: &[usize], defenders: &[usize]) -> War {
    War {
        id: crate::common::WarId(id),
        name: "Test War".to_string(),
        attackers: attackers.iter().map(|&c| CountryId(c)).collect(),
        defenders: defenders.iter().map(|&c| CountryId(c)).collect(),
        primary_attacker: attackers.first().map(|&c| CountryId(c)),
        primary_defender: defenders.first().map(|&c| CountryId(c)),
        source_crisis_id: None,
        war_goals: vec![],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: Default::default(),
        status: crate::war::data::WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: vec![],
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: Default::default(),
    }
}

fn raw_justification(
    id: usize,
    initiator: usize,
    target: usize,
    target_state: usize,
    is_ready: bool,
) -> WarJustification {
    WarJustification {
        id,
        initiator: CountryId(initiator),
        target: CountryId(target),
        target_state: StateId(target_state),
        start_date: "1800/01/01".to_string(),
        required_days: 30,
        days_passed: 0,
        is_ready,
        source_crisis_id: None,
        committed_attackers: vec![],
        committed_defenders: vec![],
    }
}

fn unresolved_crisis(id: usize, initiator: usize, target: usize) -> DiplomaticCrisis {
    DiplomaticCrisis {
        id: crate::common::DiplomaticCrisisId(id),
        initiator: CountryId(initiator),
        target: CountryId(target),
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
        related_claim_id: None,
        related_justification_id: None,
        related_war_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
// A. cooldown純粋関数 (要求テスト1-8)
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト1: Claim履歴なしなら生成可能。
#[test]
fn no_history_permits_generation() {
    assert!(claim_generation_cooldown_satisfied(None, 100));
}

/// 要求テスト2-4: 11か月では不可、ちょうど12か月・13か月以上では可能。
#[test]
fn cooldown_boundaries_11_12_13_months() {
    let current = 100;
    assert!(!claim_generation_cooldown_satisfied(
        Some(current - 11),
        current
    ));
    assert!(claim_generation_cooldown_satisfied(
        Some(current - 12),
        current
    ));
    assert!(claim_generation_cooldown_satisfied(
        Some(current - 13),
        current
    ));
}

/// 要求テスト5: 未来日付のClaim(月indexが現在より大きい)があれば生成不可。
#[test]
fn future_dated_claim_blocks_generation() {
    assert!(!claim_generation_cooldown_satisfied(Some(105), 100));
}

/// 要求テスト6: 不正な日付文字列を見つけてもpanicせず、cooldown不成立(安全側)として扱う。
#[test]
fn invalid_created_date_does_not_panic_and_blocks_cooldown() {
    let month_idx = claim_month_index_or_blocking("not-a-date", 500);
    assert_eq!(
        month_idx, 500,
        "unparsable date must fall back to the current month index (blocking)"
    );
}

/// 要求テスト7-8: Active/Consumed両方、ClaimSourceを問わずcooldown計算対象になる。
#[test]
fn cooldown_history_covers_active_and_consumed_regardless_of_source() {
    let mut registry = ClaimRegistry::default();
    let mut c1 = claim(0, 0, 1, "1800/06/01");
    c1.status = ClaimStatus::Consumed;
    c1.source = ClaimSource::AiExpansion;
    registry.claims.insert(c1.id, c1);
    let mut c2 = claim(1, 0, 2, "1801/01/01"); // より新しい
    c2.status = ClaimStatus::Active;
    c2.source = ClaimSource::Historical;
    registry.claims.insert(c2.id, c2);

    let current_month_index = month_index(1802, 1); // 1801/01からちょうど12か月後
    let (active, latest) = collect_claim_history(&registry, current_month_index);
    assert!(active.contains(&CountryId(0)));
    assert_eq!(
        latest.get(&CountryId(0)).copied(),
        Some(month_index(1801, 1))
    );
}

// ─────────────────────────────────────────────────────────────────────
// P21-019接続: 講和(`ClaimRegistry::mark_consumed`)後のcooldown・再生成挙動
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト43,44,46: `mark_consumed`(P21-019の講和経路が呼ぶのと全く同じAPI)で
/// Consumedになった元Claimがある国は、(a)11か月では新規Claimを生成できず、
/// (b)ちょうど12か月で生成できるようになり、(c)Active Claim由来のブロックは
/// (statusがConsumedへ変わった時点で)講和直後から既に解除されている。
#[test]
fn post_peace_consumed_claim_cooldown_boundaries_and_active_block_release() {
    let mut env = std_env(); // claimant=0, target=1, adjacent, opinion/power条件OK
    let mut old_claim = claim(0, 0, 5, "1801/01/01");
    old_claim.status = ClaimStatus::Active;
    env.claims.claims.insert(old_claim.id, old_claim);

    // P21-019のexecute_peace_settlementが呼ぶのと同一のドメインAPI。
    assert!(env.claims.mark_consumed(ClaimId(0)));
    assert_eq!(
        env.claims.claims.get(&ClaimId(0)).unwrap().status,
        ClaimStatus::Consumed
    );

    // (c) Active Claimブロックは講和直後から解除されている(cooldownはまだ未満了)。
    let claimants_with_active_claim: std::collections::HashSet<CountryId> = env
        .claims
        .claims
        .values()
        .filter(|c| c.status == ClaimStatus::Active)
        .map(|c| c.claimant_country)
        .collect();
    assert!(!claimants_with_active_claim.contains(&CountryId(0)));

    // (a) 11か月後(1801/01→1801/12)ではまだ生成不可。
    let grouped_11 = collect_at(&env, None, "1801/12/01", month_index(1801, 12));
    assert!(
        !grouped_11.contains_key(&CountryId(0)),
        "11 months after the consumed claim's created_date, cooldown must still block generation"
    );

    // (b) ちょうど12か月後(1802/01)では生成可能。
    let grouped_12 = collect_at(&env, None, "1802/01/01", month_index(1802, 1));
    assert!(
        grouped_12.contains_key(&CountryId(0)),
        "exactly 12 months after the consumed claim's created_date, generation must be allowed"
    );
}

/// 要求テスト45: cooldownの基準は講和が成立した日付ではなく、元Claimの
/// `created_date`のまま(`mark_consumed`は`status`だけを書き換え、
/// `created_date`を講和日へ更新しない)。
#[test]
fn mark_consumed_does_not_rewrite_created_date_and_cooldown_uses_the_original_date() {
    let mut env = std_env();
    let mut old_claim = claim(0, 0, 5, "1801/01/01");
    old_claim.status = ClaimStatus::Active;
    env.claims.claims.insert(old_claim.id, old_claim);
    env.claims.mark_consumed(ClaimId(0));

    let stored = env.claims.claims.get(&ClaimId(0)).unwrap();
    assert_eq!(
        stored.created_date, "1801/01/01",
        "mark_consumed must not rewrite created_date to the settlement date"
    );

    // 講和が(仮に)ずっと後の日付で成立していたとしても、cooldownは
    // created_date(1801/01/01)からの経過月数で計算される。
    let settlement_month_index = month_index(1801, 6); // 講和が5か月後に成立したと仮定
    let claim_month_idx = claim_month_index_or_blocking("1801/01/01", settlement_month_index);
    assert_eq!(claim_month_idx, month_index(1801, 1));
}

// ─────────────────────────────────────────────────────────────────────
// B. AI国家側の生成資格 (要求テスト9-17)
// ─────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn eligible_country_gate_inputs() -> (
    HashMap<CountryId, u32>,
    HashSet<CountryId>,
    HashSet<CountryId>,
    HashSet<CountryId>,
    HashSet<CountryId>,
    CountryPowerRegistry,
    HashMap<CountryId, i32>,
) {
    let mut owned = HashMap::new();
    owned.insert(CountryId(0), 1);
    (
        owned,
        HashSet::new(),
        HashSet::new(),
        HashSet::new(),
        HashSet::new(),
        power_registry_with(&[(0, 50.0)]),
        HashMap::new(),
    )
}

/// 要求テスト9: 障害なしのAI国家は資格あり。
#[test]
fn ai_country_without_blockers_is_eligible() {
    let (owned, active, crisis, just, war, power, history) = eligible_country_gate_inputs();
    assert!(country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト10: プレイヤー国家は評価しない。
#[test]
fn player_country_is_never_eligible() {
    let (owned, active, crisis, just, war, power, history) = eligible_country_gate_inputs();
    assert!(!country_can_generate_claim(
        CountryId(0),
        Some(CountryId(0)),
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト11: 所有陸州なしでは生成しない。
#[test]
fn country_without_owned_land_states_is_not_eligible() {
    let (_, active, crisis, just, war, power, history) = eligible_country_gate_inputs();
    let owned = HashMap::new(); // 空
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト12: Active Claim保有中は生成しない。
#[test]
fn country_with_active_claim_is_not_eligible() {
    let (owned, _, crisis, just, war, power, history) = eligible_country_gate_inputs();
    let mut active = HashSet::new();
    active.insert(CountryId(0));
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト13: 未解決の自国開始Crisis中は生成しない。
#[test]
fn country_with_unresolved_self_crisis_is_not_eligible() {
    let (owned, active, _, just, war, power, history) = eligible_country_gate_inputs();
    let mut crisis = HashSet::new();
    crisis.insert(CountryId(0));
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト14-15: 進行中(is_ready=false)・Ready(is_ready=true)いずれの正当化中も
/// 生成しない(集合はis_readyを区別しないため、両者が同じ経路で除外されることを示す)。
#[test]
fn country_with_any_justification_status_is_not_eligible() {
    let (owned, active, crisis, _, war, power, history) = eligible_country_gate_inputs();
    let mut just = HashSet::new();
    just.insert(CountryId(0));
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &power,
        &history,
        1000,
    ));

    let mut jreg = WarJustificationRegistry::default();
    jreg.justifications
        .insert(0, raw_justification(0, 0, 1, 1, false));
    jreg.justifications
        .insert(1, raw_justification(1, 2, 3, 3, true));
    let initiators = collect_justification_initiators(&jreg);
    assert!(
        initiators.contains(&CountryId(0)),
        "in-progress justification initiator must be tracked"
    );
    assert!(
        initiators.contains(&CountryId(2)),
        "ready justification initiator must be tracked identically"
    );
}

/// 要求テスト16: War参加中(攻撃側・防御側いずれか)は生成しない。
#[test]
fn country_at_war_is_not_eligible() {
    let (owned, active, crisis, just, _, power, history) = eligible_country_gate_inputs();
    let mut war_registry = WarRegistry::default();
    war_registry.add_war(active_war(0, &[0], &[9]));
    let war_participants = collect_war_participants(&war_registry);
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war_participants,
        &power,
        &history,
        1000,
    ));
}

/// 要求テスト17: CountryPowerRegistry欠落時にpanicせず資格なしとして扱う。
#[test]
fn missing_power_registry_entry_does_not_panic() {
    let (owned, active, crisis, just, war, _, history) = eligible_country_gate_inputs();
    let empty_power = CountryPowerRegistry::default();
    assert!(!country_can_generate_claim(
        CountryId(0),
        None,
        true,
        &owned,
        &active,
        &crisis,
        &just,
        &war,
        &empty_power,
        &history,
        1000,
    ));
}

// ─────────────────────────────────────────────────────────────────────
// C. 候補構築 (collect_ai_claim_candidatesを通した統合的な検証)
// ─────────────────────────────────────────────────────────────────────

struct StdEnv {
    claims: ClaimRegistry,
    crises: CrisisRegistry,
    justifications: WarJustificationRegistry,
    countries: CountryRegistry,
    states: StateRegistry,
    diplomacy: DiplomacyRegistry,
    wars: WarRegistry,
    power: CountryPowerRegistry,
}

/// claimant=0(強国、州0所有・州1に隣接)、target=1(弱国、州1所有)。
/// opinion=-30(基準を満たす)、power比は10倍(基準を満たす)。
fn std_env() -> StdEnv {
    let countries = CountryRegistry {
        countries: vec![country(0), country(1)],
    };
    let states = StateRegistry::build(vec![land_state(0, 0, &[1]), land_state(1, 1, &[0])]);
    let mut diplomacy = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy, 0, 1, -30.0);
    StdEnv {
        claims: ClaimRegistry::default(),
        crises: CrisisRegistry::default(),
        justifications: WarJustificationRegistry::default(),
        countries,
        states,
        diplomacy,
        wars: WarRegistry::default(),
        power: power_registry_with(&[(0, 100.0), (1, 10.0)]),
    }
}

fn collect(env: &StdEnv, player: Option<CountryId>) -> HashMap<CountryId, Vec<AiClaimCandidate>> {
    collect_at(env, player, "1801/01/01", month_index(1801, 1))
}

/// P21-019接続テスト用: 任意の日付・月indexで`collect_ai_claim_candidates`を呼ぶ版。
fn collect_at(
    env: &StdEnv,
    player: Option<CountryId>,
    current_date: &str,
    current_month_index: i32,
) -> HashMap<CountryId, Vec<AiClaimCandidate>> {
    collect_ai_claim_candidates(
        &env.claims,
        &env.crises,
        &env.justifications,
        &env.countries,
        &env.states,
        &env.diplomacy,
        &env.wars,
        &env.power,
        player,
        current_date,
        current_month_index,
    )
}

/// 要求テスト18: 隣接する外国陸州を候補にする。
#[test]
fn adjacent_foreign_land_state_is_a_candidate() {
    let env = std_env();
    let grouped = collect(&env, None);
    let candidates = grouped
        .get(&CountryId(0))
        .expect("claimant 0 must have candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].target_state, StateId(1));
    assert_eq!(candidates[0].target, CountryId(1));
}

/// 要求テスト19: 非隣接州を候補にしない。
#[test]
fn non_adjacent_state_is_not_a_candidate() {
    let mut env = std_env();
    env.states = StateRegistry::build(vec![
        land_state(0, 0, &[]), // 州1に隣接しない
        land_state(1, 1, &[]),
    ]);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト20: 海州を候補にしない。
#[test]
fn sea_state_is_not_a_candidate() {
    let mut env = std_env();
    env.states = StateRegistry::build(vec![land_state(0, 0, &[1]), sea_state(1, 1, &[0])]);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト21: 自国所有州を候補にしない。
#[test]
fn own_state_is_not_a_candidate() {
    let mut env = std_env();
    // 州0・州1とも claimant(0) 所有 = 国境そのものが存在しない
    env.states = StateRegistry::build(vec![land_state(0, 0, &[1]), land_state(1, 0, &[0])]);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト22: 判定にはowner(法的所有国)を使い、controllerは使わない
/// (占領中の州でも法的所有国に対する候補として扱う)。
#[test]
fn owner_not_controller_determines_candidate_target() {
    let mut env = std_env();
    let mut occupied = land_state(1, 1, &[0]);
    occupied.controller_country = Some(CountryId(0)); // claimant自身が占領中
    env.states = StateRegistry::build(vec![land_state(0, 0, &[1]), occupied]);
    let grouped = collect(&env, None);
    let candidates = grouped
        .get(&CountryId(0))
        .expect("must still be a candidate against owner(1)");
    assert_eq!(
        candidates[0].target,
        CountryId(1),
        "target must be the legal owner, not the controller"
    );
}

/// 要求テスト23: 存在しないowner国家を候補にしない。
#[test]
fn nonexistent_owner_country_is_not_a_candidate() {
    let mut env = std_env();
    env.states = StateRegistry::build(vec![
        land_state(0, 0, &[1]),
        land_state(1, 99, &[0]), // CountryId(99)は未登録
    ]);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// レビュー指摘対応(§10の`at_war_pairs`最適化): claimant自身が参加していない
/// (第三国同士の)多国間Warが存在しても、候補生成には一切影響しない。これは
/// `country_can_generate_claim`の`war_participants`ゲート(claimantを含まない)を
/// 通過した時点で、`evaluate_pair_eligibility`へ渡す`at_war_pairs`を空集合化する
/// 最適化(claimantがどのWarのattackers/defendersにも登場しないため、実際の
/// ペア集合を展開しても`.contains(&normalize_pair(claimant,target))`は常にfalse
/// になる)が安全であることを、実際の`WarRegistry`経由で確認する。
#[test]
fn unrelated_multilateral_war_does_not_affect_candidate_generation() {
    let mut env = std_env();
    let mut war_registry = WarRegistry::default();
    // claimant(0)・target(1)のどちらも参加していない、無関係な多国間War。
    war_registry.add_war(active_war(0, &[8, 9], &[10, 11]));
    env.wars = war_registry;

    let grouped = collect(&env, None);
    assert!(
        grouped.contains_key(&CountryId(0)),
        "an unrelated multilateral war must not block claim generation for uninvolved countries"
    );
}

/// 要求テスト24: `evaluate_pair_eligibility`(P21-017と共有)は戦争中ペアを拒否する
/// (`ai_crisis_start`側で網羅的にテスト済みの共有関数を、このモジュールが実際に
/// 呼び出していることを確認する)。
#[test]
fn evaluate_pair_eligibility_rejects_pair_at_war() {
    let diplomacy = DiplomacyRegistry::default();
    let mut at_war = HashSet::new();
    at_war.insert((CountryId(0), CountryId(1)));
    let power = power_registry_with(&[(0, 100.0), (1, 10.0)]);
    let result = evaluate_pair_eligibility(
        CountryId(0),
        CountryId(1),
        &diplomacy,
        &at_war,
        &power,
        "1801/01/01",
    );
    assert_eq!(result, None);
}

/// 要求テスト25: 同盟国の州を候補にしない。
#[test]
fn allied_target_state_is_not_a_candidate() {
    let mut env = std_env();
    set_treaty(&mut env.diplomacy, 0, 1, TreatyType::Alliance);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト26: 不可侵条約国の州を候補にしない。
#[test]
fn non_aggression_pact_target_state_is_not_a_candidate() {
    let mut env = std_env();
    set_treaty(&mut env.diplomacy, 0, 1, TreatyType::NonAggressionPact);
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト27: 休戦中の国の州を候補にしない。
#[test]
fn truce_target_state_is_not_a_candidate() {
    let mut env = std_env();
    env.diplomacy
        .set_truce(CountryId(0), CountryId(1), "1900/01/01".to_string());
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト28-29: opinion境界(-25.0は候補、-24.9は候補外)。
#[test]
fn opinion_boundary() {
    let mut env = std_env();
    set_opinion(&mut env.diplomacy, 0, 1, -25.0);
    assert!(collect(&env, None).contains_key(&CountryId(0)));

    let mut env2 = std_env();
    set_opinion(&mut env2.diplomacy, 0, 1, -24.9);
    assert!(!collect(&env2, None).contains_key(&CountryId(0)));
}

/// 要求テスト30-31: power ratio境界(0.75は候補、0.749は候補外)。
#[test]
fn power_ratio_boundary() {
    let mut env = std_env();
    env.power = power_registry_with(&[(0, 75.0), (1, 100.0)]); // ratio = 0.75
    assert!(collect(&env, None).contains_key(&CountryId(0)));

    let mut env2 = std_env();
    env2.power = power_registry_with(&[(0, 74.9), (1, 100.0)]); // ratio = 0.749
    assert!(!collect(&env2, None).contains_key(&CountryId(0)));
}

/// 要求テスト32: 既にActive Claimを保有する国は(そのClaimの対象がどこであれ)
/// 新規候補を一切生成しない — 「同じ州へのActive Claim」よりも強い、
/// 国家単位の排他条件で保証される。
#[test]
fn country_holding_any_active_claim_gets_no_new_candidates() {
    let mut env = std_env();
    env.claims.add_claim(claim(0, 0, 1, "1800/01/01"));
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト33: 自国開始の未解決Crisisがあれば候補を生成しない。
#[test]
fn unresolved_self_crisis_blocks_all_candidates_for_claimant() {
    let mut env = std_env();
    env.crises.add_crisis(unresolved_crisis(0, 0, 5));
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト34: 進行中/Ready正当化があれば候補を生成しない。
#[test]
fn justification_blocks_all_candidates_for_claimant() {
    let mut env = std_env();
    env.justifications
        .justifications
        .insert(0, raw_justification(0, 0, 5, 5, true));
    let grouped = collect(&env, None);
    assert!(!grouped.contains_key(&CountryId(0)));
}

/// 要求テスト35: 対象国の首都州も、既存API(`can_create_claim`)が禁止していない限り
/// 候補になり得る。
#[test]
fn capital_state_can_be_a_candidate() {
    let mut env = std_env();
    let mut target = country(1);
    target.capital_state_id = StateId(1);
    env.countries = CountryRegistry {
        countries: vec![country(0), target],
    };
    let grouped = collect(&env, None);
    assert!(
        grouped.contains_key(&CountryId(0)),
        "capital state must remain a valid candidate"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D. 候補選択順位 (要求テスト36-44)
// ─────────────────────────────────────────────────────────────────────

fn multi_target_env() -> StdEnv {
    // claimant(0)は州0を所有し、州1(target 1)・州2(target 2)双方に隣接する。
    let countries = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    let states = StateRegistry::build(vec![
        land_state(0, 0, &[1, 2]),
        land_state(1, 1, &[0]),
        land_state(2, 2, &[0]),
    ]);
    let mut diplomacy = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy, 0, 1, -30.0);
    set_opinion(&mut diplomacy, 0, 2, -30.0);
    StdEnv {
        claims: ClaimRegistry::default(),
        crises: CrisisRegistry::default(),
        justifications: WarJustificationRegistry::default(),
        countries,
        states,
        diplomacy,
        wars: WarRegistry::default(),
        power: power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]),
    }
}

/// 要求テスト36: opinionが最も低い候補が選ばれる。
#[test]
fn lowest_opinion_candidate_is_selected_first() {
    let mut env = multi_target_env();
    set_opinion(&mut env.diplomacy, 0, 1, -30.0);
    set_opinion(&mut env.diplomacy, 0, 2, -80.0);
    let grouped = collect(&env, None);
    let candidates = &grouped[&CountryId(0)];
    assert_eq!(candidates[0].target, CountryId(2));
}

/// 要求テスト37: opinion同値ならpower ratioが高い候補。
#[test]
fn tied_opinion_breaks_by_higher_power_ratio() {
    let mut env = multi_target_env();
    env.power = power_registry_with(&[(0, 100.0), (1, 50.0), (2, 10.0)]); // 2の方が比が高い
    let grouped = collect(&env, None);
    let candidates = &grouped[&CountryId(0)];
    assert_eq!(candidates[0].target, CountryId(2));
}

/// 要求テスト38: opinion・power ratio同値なら人口が多い候補。
#[test]
fn tied_through_power_ratio_breaks_by_higher_population() {
    let mut env = multi_target_env();
    env.states = StateRegistry::build(vec![
        land_state(0, 0, &[1, 2]),
        StateData {
            population: 1000,
            ..land_state(1, 1, &[0])
        },
        StateData {
            population: 5000,
            ..land_state(2, 2, &[0])
        },
    ]);
    let grouped = collect(&env, None);
    let candidates = &grouped[&CountryId(0)];
    assert_eq!(candidates[0].target_state, StateId(2));
}

/// 要求テスト39: さらに同値なら建物レベル合計が高い候補。
#[test]
fn tied_through_population_breaks_by_building_level_sum() {
    use crate::building::data::BuildingType;
    let mut env = multi_target_env();
    let mut s1 = land_state(1, 1, &[0]);
    s1.buildings.insert(BuildingType::Farm, 2);
    let mut s2 = land_state(2, 2, &[0]);
    s2.buildings.insert(BuildingType::Farm, 5);
    env.states = StateRegistry::build(vec![land_state(0, 0, &[1, 2]), s1, s2]);
    let grouped = collect(&env, None);
    let candidates = &grouped[&CountryId(0)];
    assert_eq!(candidates[0].target_state, StateId(2));
}

/// 要求テスト40-41: 完全同値ならCountryId昇順、さらにStateId昇順。
#[test]
fn fully_tied_candidates_break_by_country_id_then_state_id() {
    let env = multi_target_env(); // opinion/power ratio/population/buildingsが全て同一
    let grouped = collect(&env, None);
    let candidates = &grouped[&CountryId(0)];
    assert_eq!(candidates[0].target, CountryId(1));
    assert_eq!(candidates[0].target_state, StateId(1));
}

/// 要求テスト42: State/隣接リストの挿入順を変えても同じ結果になる(決定論性)。
#[test]
fn candidate_order_is_independent_of_state_insertion_order() {
    let mut env_b = multi_target_env();
    // 州の挿入順を入れ替え、neighborsの列挙順も逆にする
    env_b.states = StateRegistry::build(vec![
        land_state(2, 2, &[0]),
        land_state(1, 1, &[0]),
        land_state(0, 0, &[2, 1]),
    ]);
    set_opinion(&mut env_b.diplomacy, 0, 2, -80.0);
    let mut env_a2 = multi_target_env();
    set_opinion(&mut env_a2.diplomacy, 0, 2, -80.0);

    let grouped_a = collect(&env_a2, None);
    let grouped_b = collect(&env_b, None);
    assert_eq!(
        grouped_a[&CountryId(0)][0].target_state,
        grouped_b[&CountryId(0)][0].target_state
    );
}

/// 要求テスト43: 1回の呼び出しで1国は最大1Claimしか生成しない。
#[test]
fn evaluate_ai_claim_generation_creates_at_most_one_claim_per_country_per_call() {
    let mut env = multi_target_env();
    let player = PlayerCountry(None);
    evaluate_ai_claim_generation(
        &mut env.claims,
        &env.crises,
        &env.justifications,
        &env.countries,
        &env.states,
        &env.diplomacy,
        &env.wars,
        &env.power,
        &player,
        "1801/01/01",
        1801,
        1,
    );
    let claimant_claims: Vec<_> = env
        .claims
        .claims
        .values()
        .filter(|c| c.claimant_country == CountryId(0))
        .collect();
    assert_eq!(claimant_claims.len(), 1);
}

/// 要求テスト44: 複数AIはCountryId昇順で処理される(双方とも資格を満たす場合、
/// 双方に1件ずつClaimが生成されることで間接的に確認する)。
#[test]
fn multiple_ai_countries_are_each_processed() {
    let countries = CountryRegistry {
        countries: vec![country(0), country(1), country(2), country(3)],
    };
    let states = StateRegistry::build(vec![
        land_state(0, 0, &[1]),
        land_state(1, 1, &[0]),
        land_state(2, 2, &[3]),
        land_state(3, 3, &[2]),
    ]);
    let mut diplomacy = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy, 0, 1, -30.0);
    set_opinion(&mut diplomacy, 2, 3, -30.0);
    let mut claims = ClaimRegistry::default();
    let crises = CrisisRegistry::default();
    let justifications = WarJustificationRegistry::default();
    let wars = WarRegistry::default();
    let power = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 100.0), (3, 10.0)]);
    let player = PlayerCountry(None);

    evaluate_ai_claim_generation(
        &mut claims,
        &crises,
        &justifications,
        &countries,
        &states,
        &diplomacy,
        &wars,
        &power,
        &player,
        "1801/01/01",
        1801,
        1,
    );

    assert!(claims.has_claim(CountryId(0), StateId(1)));
    assert!(claims.has_claim(CountryId(2), StateId(3)));
}

// ─────────────────────────────────────────────────────────────────────
// E. Claim作成ドメインAPI (要求テスト45-53)
// ─────────────────────────────────────────────────────────────────────

/// 要求テスト45,52: 候補が既存のClaimと衝突する場合(直前再検証で失敗)、次の候補へ
/// フォールバックし、Registryへ部分的な書き込みを残さない(ID重複も発生しない)。
#[test]
fn first_candidate_domain_failure_falls_through_without_partial_writes() {
    let mut claim_registry = ClaimRegistry::default();
    let countries = CountryRegistry {
        countries: vec![country(0), country(1), country(2)],
    };
    let states = StateRegistry::build(vec![land_state(1, 1, &[]), land_state(2, 2, &[])]);

    let candidates = vec![
        AiClaimCandidate {
            target_state: StateId(999), // 存在しない州: state_missingで失敗
            target: CountryId(1),
            opinion: -50.0,
            power_ratio: 2.0,
            population: 0,
            building_level_sum: 0,
        },
        AiClaimCandidate {
            target_state: StateId(2),
            target: CountryId(2),
            opinion: -40.0,
            power_ratio: 2.0,
            population: 0,
            building_level_sum: 0,
        },
    ];

    let result = try_create_claim_from_candidates(
        &mut claim_registry,
        &countries,
        &states,
        &candidates,
        CountryId(0),
        "1801/01/01",
    );

    assert!(
        result.is_some(),
        "must fall through to the second, valid candidate"
    );
    assert_eq!(
        claim_registry.claims.len(),
        1,
        "the failed first attempt must not leave a partial write"
    );
    let created = claim_registry.claims.values().next().unwrap();
    assert_eq!(created.target_state, StateId(2));
    assert_eq!(created.source, ClaimSource::AiExpansion);
}

/// 要求テスト46: 同じClaimant・州のActive Claimを重複生成しない
/// (`create_claim`自身が保証する不変条件を、このモジュールの呼び出し経路で再確認する)。
#[test]
fn duplicate_active_claim_is_never_created_via_this_module() {
    let mut claim_registry = ClaimRegistry::default();
    let countries = CountryRegistry {
        countries: vec![country(0), country(1)],
    };
    let states = StateRegistry::build(vec![land_state(0, 0, &[]), land_state(1, 1, &[])]);
    claim_registry
        .create_claim(
            CountryId(0),
            CountryId(1),
            StateId(1),
            "1800/01/01".to_string(),
            ClaimSource::AiExpansion,
            &countries,
            &states,
        )
        .unwrap();

    let candidates = vec![AiClaimCandidate {
        target_state: StateId(1),
        target: CountryId(1),
        opinion: -50.0,
        power_ratio: 2.0,
        population: 0,
        building_level_sum: 0,
    }];
    let result = try_create_claim_from_candidates(
        &mut claim_registry,
        &countries,
        &states,
        &candidates,
        CountryId(0),
        "1801/01/01",
    );
    assert_eq!(result, None);
    assert_eq!(claim_registry.claims.len(), 1);
}

/// 要求テスト53: AI生成もプレイヤーUI(`diplomacy_panel`)と全く同じ
/// `ClaimRegistry::create_claim`ドメインAPIを経由する(シグネチャ・検証ロジックの共有)。
#[test]
fn ai_and_player_style_claim_creation_share_the_same_domain_api() {
    let countries = CountryRegistry {
        countries: vec![country(0), country(1)],
    };
    let states = StateRegistry::build(vec![land_state(0, 0, &[]), land_state(1, 1, &[])]);
    let mut registry = ClaimRegistry::default();

    // プレイヤーUI(`diplomacy_panel::execute_claim_create`)と全く同じ呼び出し形。
    registry
        .create_claim(
            CountryId(0),
            CountryId(1),
            StateId(1),
            "1800/01/01".to_string(),
            ClaimSource::Strategic,
            &countries,
            &states,
        )
        .unwrap();

    // AI生成側は同じ関数をClaimSource::AiExpansionで呼ぶだけで、対象州を変えれば成功する。
    let states2 = StateRegistry::build(vec![land_state(0, 0, &[]), land_state(2, 1, &[])]);
    let candidates = vec![AiClaimCandidate {
        target_state: StateId(2),
        target: CountryId(1),
        opinion: -50.0,
        power_ratio: 2.0,
        population: 0,
        building_level_sum: 0,
    }];
    let result = try_create_claim_from_candidates(
        &mut registry,
        &countries,
        &states2,
        &candidates,
        CountryId(0),
        "1801/01/01",
    );
    assert!(result.is_some());
    assert_eq!(registry.claims.len(), 2);
}

// ─────────────────────────────────────────────────────────────────────
// F. 月次Systemと順序 (要求テスト54-59)
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
        land_state(0, 0, &[1]),
        land_state(1, 1, &[0]),
    ]));
    let mut diplomacy_registry = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy_registry, 0, 1, -30.0);
    app.insert_resource(diplomacy_registry);
    app.insert_resource(WarRegistry::default());
    app.insert_resource(power_registry_with(&[(0, 100.0), (1, 10.0)]));
    app.insert_resource(PlayerCountry(player));
    app.insert_resource(GameDate::new(1800, 2, 1));
    app.insert_resource(AiClaimGenerationEvaluationCount::default());
    app.add_systems(Update, handle_monthly_ai_claim_generation);
    app
}

/// 要求テスト54: `MonthChangedMessage`なしでは評価しない。
#[test]
fn no_month_changed_message_means_no_evaluation() {
    let mut app = build_test_app(None);
    app.update();
    assert!(app.world().resource::<ClaimRegistry>().claims.is_empty());
    assert_eq!(
        app.world().resource::<AiClaimGenerationEvaluationCount>().0,
        0
    );
}

/// 要求テスト55: 月変更時に評価する。
#[test]
fn month_changed_message_triggers_evaluation() {
    let mut app = build_test_app(None);
    app.world_mut().write_message(MonthChangedMessage {
        year: 1800,
        month: 2,
    });
    app.update();
    assert_eq!(app.world().resource::<ClaimRegistry>().claims.len(), 1);
    assert_eq!(
        app.world().resource::<AiClaimGenerationEvaluationCount>().0,
        1
    );
}

/// 要求テスト56-57: 同一フレームに複数`MonthChangedMessage`があっても評価は
/// フレームにつき最大1回(実際の呼び出し回数で検証)。
#[test]
fn multiple_month_changed_messages_in_one_frame_evaluate_only_once() {
    let mut app = build_test_app(None);
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
        app.world().resource::<AiClaimGenerationEvaluationCount>().0,
        1
    );
    assert_eq!(app.world().resource::<ClaimRegistry>().claims.len(), 1);
}

/// 要求テスト58: Pause中相当(`advance_game_date`自体を実行しない=MonthChangedMessageが
/// 発行されない)では複数フレーム進めても生成しない。
#[test]
fn no_message_means_no_generation_across_multiple_frames() {
    let mut app = build_test_app(None);
    app.update();
    app.update();
    app.update();
    assert_eq!(
        app.world().resource::<AiClaimGenerationEvaluationCount>().0,
        0
    );
    assert!(app.world().resource::<ClaimRegistry>().claims.is_empty());
}

/// 要求テスト59: Load直後(MonthChangedMessage無し)だけでは生成しない。
#[test]
fn load_without_a_month_changed_message_does_not_generate() {
    let mut app = build_test_app(None);
    // Load直後相当: 既存Claim(cooldown対象)を持つ状態を復元したと仮定しても、
    // MonthChangedMessageが発行されない限り評価は起きない。
    app.world_mut()
        .resource_mut::<ClaimRegistry>()
        .add_claim(claim(1, 1, 0, "1799/01/01"));
    app.update();
    // claimant=1のClaimはそのまま(claimant=0側は新規未生成)。
    assert_eq!(app.world().resource::<ClaimRegistry>().claims.len(), 1);
}

/// プレイヤー国家はSystem経由でも評価されない。
#[test]
fn player_country_is_not_evaluated_via_system() {
    let mut app = build_test_app(Some(CountryId(0)));
    app.world_mut().write_message(MonthChangedMessage {
        year: 1800,
        month: 2,
    });
    app.update();
    assert!(app.world().resource::<ClaimRegistry>().claims.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
// G. §11のSystem順序が保証する、同フレーム内の他AI経路との競合防止
// (要求テスト63-64)。`handle_daily_country_ai`のSystemそのものではなく、
// それが呼ぶ純粋関数`process_daily_country_ai`を直接呼び、その結果として
// 変異したRegistryをそのまま`evaluate_ai_claim_generation`へ渡すことで、
// 「P21-018が同フレーム内の直前の書き込みを正しく読む」というデータレベルの
// 性質を検証する(Bevyスケジューラ自体の`.after()`遵守は§11のSystem登録+
// フレームワークの責務であり、ここでは再検証しない)。
// ─────────────────────────────────────────────────────────────────────

/// process_daily_country_aiが呼ぶ各種Registryのための最小限のデフォルト値。
#[allow(clippy::type_complexity)]
fn country_ai_extra_registries() -> (
    crate::building::data::BuildingRegistry,
    crate::military::data::MilitaryRegistry,
    crate::research::data::TechnologyRegistry,
    crate::research::world_stage::WorldCivilizationState,
    crate::war::frontline::FrontlineRegistry,
    crate::war::military_ai::MilitaryAiRegistry,
    CountryAiRegistry,
) {
    (
        crate::building::data::BuildingRegistry::default(),
        crate::military::data::MilitaryRegistry::default(),
        crate::research::data::TechnologyRegistry::default(),
        crate::research::world_stage::WorldCivilizationState::default(),
        crate::war::frontline::FrontlineRegistry::default(),
        crate::war::military_ai::MilitaryAiRegistry::default(),
        CountryAiRegistry::default(),
    )
}

/// 要求テスト63: 通常戦争準備AI(`process_war_preparation_ai`、
/// `process_daily_country_ai`が週次評価として呼ぶ)が同フレームで新しい正当化を
/// 開始したAIは、直後に評価される`evaluate_ai_claim_generation`ではClaimを
/// 生成しない。Strong(0)はWeak(1)への正当化(隣接・戦力優位)と、
/// ClaimTarget(2)へのClaim生成(隣接・opinion/power ratio条件を満たす)の
/// 両方の条件を満たすが、`process_daily_country_ai`が先に実行されて正当化を
/// 作るため、同じ変異後のRegistryを読む`evaluate_ai_claim_generation`は
/// Strong(0)を`justification_initiators`により除外する。
#[test]
fn war_prep_ai_justification_started_this_frame_preempts_claim_generation_for_the_same_country() {
    let strong = country(0);
    let weak = country(1);
    let claim_target = country(2);
    let mut country_registry = CountryRegistry {
        countries: vec![strong, weak, claim_target],
    };

    let s0 = StateData {
        neighbors: vec![StateId(1), StateId(2)],
        ..land_state(0, 0, &[1, 2])
    };
    let s1 = land_state(1, 1, &[0]);
    let s2 = land_state(2, 2, &[0]);
    let state_registry = StateRegistry::build(vec![s0, s1, s2]);

    let mut military_registry = crate::military::data::MilitaryRegistry::default();
    // country_ai.rsのtest_war_preparation_ai_starts_justification_via_optimized_pathと
    // 同じ、Strong(0)がWeak(1)へ130%以上の戦力優位を持つ設定。
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    let make_division = |owner: CountryId, current_state: StateId, manpower: u64| Division {
        id: crate::common::DivisionId(0),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
        destination: None,
        current_path: Vec::new(),
        target_state: None,
        manpower,
        max_manpower: manpower,
        equipment: 10.0,
        max_equipment: 10.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 1.0,
        max_morale: 1.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: crate::common::DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    };
    military_registry.add_division(make_division(CountryId(0), StateId(0), 10_000));
    military_registry.add_division(make_division(CountryId(1), StateId(1), 1_000));

    let mut war_registry = WarRegistry::default();
    let mut diplomacy_registry = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy_registry, 0, 2, -50.0); // Claim生成条件(§6)を満たす
    let mut justification_registry = WarJustificationRegistry::default();
    let mut crisis_registry = CrisisRegistry::default();
    let (
        building_registry,
        _mr_unused,
        tech_registry,
        world_state,
        mut frontline_registry,
        mut military_ai_registry,
        mut country_ai_registry,
    ) = country_ai_extra_registries();
    let mut pending_ai_wars = Vec::new();
    let player_country = PlayerCountry(None);

    // day=7: Strong(0)の週次評価(day % 7 == country_id % 7)を成立させ、
    // process_war_preparation_aiがWeak(1)への正当化を開始する。
    process_daily_country_ai(
        7,
        "1800/01/08",
        &player_country,
        &mut country_registry,
        &state_registry,
        &building_registry,
        &military_registry,
        &mut war_registry,
        &mut diplomacy_registry,
        &tech_registry,
        &world_state,
        &mut justification_registry,
        &mut frontline_registry,
        &mut military_ai_registry,
        &mut country_ai_registry,
        &mut pending_ai_wars,
        &mut crisis_registry,
    );
    assert!(
        justification_registry
            .justifications
            .values()
            .any(|j| j.initiator == CountryId(0)),
        "test setup must actually trigger a same-day justification for Strong(0)"
    );

    let power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]);
    let claim_registry_before = ClaimRegistry::default();
    let mut claim_registry = claim_registry_before;
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
        "1800/01/08",
        1800,
        1,
    );

    assert!(
        claim_registry.claims.is_empty(),
        "Strong(0) must not generate a claim against ClaimTarget(2) in the same frame it just \
         started a justification against Weak(1) via the ordinary war-preparation AI"
    );
}

/// 要求テスト64: AI宣戦布告(`process_war_declaration_ai`、`process_daily_country_ai`が
/// 日次評価として毎日呼ぶ)によって同フレームでWarへ入ったAIは、直後に評価される
/// `evaluate_ai_claim_generation`ではClaimを生成しない。
#[test]
fn ai_declares_war_this_frame_preempts_claim_generation_for_the_same_country() {
    let attacker = country(0);
    let defender = country(1);
    let claim_target = country(2);
    let mut country_registry = CountryRegistry {
        countries: vec![attacker, defender, claim_target],
    };

    let s0 = land_state(0, 0, &[1, 2]);
    let s1 = land_state(1, 1, &[0]);
    let s2 = land_state(2, 2, &[0]);
    let state_registry = StateRegistry::build(vec![s0, s1, s2]);

    let mut war_registry = WarRegistry::default();
    let mut diplomacy_registry = DiplomacyRegistry::default();
    set_opinion(&mut diplomacy_registry, 0, 2, -50.0);
    let mut justification_registry = WarJustificationRegistry::default();
    justification_registry
        .justifications
        .insert(0, raw_justification(0, 0, 1, 1, true)); // is_ready=true: 即宣戦可能
    let mut crisis_registry = CrisisRegistry::default();
    let (
        building_registry,
        military_registry,
        tech_registry,
        world_state,
        mut frontline_registry,
        mut military_ai_registry,
        mut country_ai_registry,
    ) = country_ai_extra_registries();
    let mut pending_ai_wars = Vec::new();
    let player_country = PlayerCountry(None);

    process_daily_country_ai(
        1,
        "1800/01/01",
        &player_country,
        &mut country_registry,
        &state_registry,
        &building_registry,
        &military_registry,
        &mut war_registry,
        &mut diplomacy_registry,
        &tech_registry,
        &world_state,
        &mut justification_registry,
        &mut frontline_registry,
        &mut military_ai_registry,
        &mut country_ai_registry,
        &mut pending_ai_wars,
        &mut crisis_registry,
    );
    assert!(
        war_registry
            .wars
            .values()
            .any(|w| w.attackers.contains(&CountryId(0))),
        "test setup must actually trigger a same-day war declaration for CountryId(0)"
    );

    let power_registry = power_registry_with(&[(0, 100.0), (1, 10.0), (2, 10.0)]);
    let mut claim_registry = ClaimRegistry::default();
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
        "1800/01/01",
        1800,
        1,
    );

    assert!(
        claim_registry.claims.is_empty(),
        "CountryId(0) must not generate a claim against ClaimTarget(2) in the same frame it just \
         declared war against CountryId(1) via the AI war-declaration path"
    );
}
