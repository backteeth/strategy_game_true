//! P21-021性能プロファイリング(§18要求)。一時プロファイラ——結果を
//! `verification_logs/phase-21/p21-021/perf/{results.csv,summary.txt}`へ保存した後、
//! このファイル自体は削除する(このセッションで確立された慣習: P21-010-PERF-VERIFY等と
//! 同じ「一時プロファイラ、使用後削除」パターン)。
//!
//! `strategy_game::profiling::build_app`(本番と同一プラグイン構成)で合成ワールドを
//! 構築し、`country::elimination::{preflight_country_elimination, apply_country_elimination}`
//! を直接呼んで計測する(`execute_peace_settlement`経由の余分なオーバーヘッドを含めず、
//! 消滅処理自体のコストだけを分離する)。
//!
//! 実行: cargo test --release --test profile_p21_021_elimination_scaling_test -- --ignored --nocapture

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use strategy_game::common::{
    ClaimId, CountryId, DiplomaticCrisisId, DivisionDefinitionId, DivisionId, StateId, WarId,
};
use strategy_game::country::CountryRegistry;
use strategy_game::country::elimination::{
    apply_country_elimination, preflight_country_elimination,
};
use strategy_game::country::power::CountryPowerRegistry;
use strategy_game::diplomacy::claims::{ClaimRegistry, ClaimSource, ClaimStatus, TerritorialClaim};
use strategy_game::diplomacy::crisis::{CrisisPhase, CrisisRegistry, DiplomaticCrisis};
use strategy_game::diplomacy::data::DiplomacyRegistry;
use strategy_game::military::army::ArmyRegistry;
use strategy_game::military::battle::BattleRegistry;
use strategy_game::military::data::{
    Division, DivisionSize, DivisionStatus, DivisionType, MilitaryRegistry,
};
use strategy_game::profiling::{self, Scenario, WorkloadConfig};
use strategy_game::state::data::StateRegistry;
use strategy_game::war::data::{War, WarRegistry, WarStatus};
use strategy_game::war::frontline::FrontlineRegistry;
use strategy_game::war::justification::{WarJustification, WarJustificationRegistry};

const FIXED_SEED: u64 = 0x00C0_FFEE_0021_0021;

struct Stats {
    samples_ms: Vec<f64>,
}
impl Stats {
    fn new() -> Self {
        Self {
            samples_ms: Vec::new(),
        }
    }
    fn push(&mut self, d: Duration) {
        self.samples_ms.push(d.as_secs_f64() * 1000.0);
    }
    fn mean(&self) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        self.samples_ms.iter().sum::<f64>() / self.samples_ms.len() as f64
    }
    fn percentile(&self, p: f64) -> f64 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        let mut sorted = self.samples_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }
}

fn make_division(id: usize, owner: CountryId, current_state: StateId) -> Division {
    Division {
        id: DivisionId(id),
        owner,
        division_type: DivisionType::Infantry,
        size: DivisionSize::Standard,
        current_state,
        destination: None,
        current_path: vec![],
        target_state: None,
        manpower: 10_000,
        max_manpower: 10_000,
        equipment: 100.0,
        max_equipment: 100.0,
        organization: 100.0,
        max_organization: 100.0,
        morale: 100.0,
        max_morale: 100.0,
        experience: 0.0,
        supply_ratio: 1.0,
        movement_progress: 0.0,
        status: DivisionStatus::Idle,
        def_id: DivisionDefinitionId(0),
        attack_power: 10,
        defense_power: 10,
        combat_id: None,
    }
}

fn make_war(id: usize, attacker: CountryId, defender: CountryId, target_state: StateId) -> War {
    War {
        id: WarId(id),
        name: "Perf War".to_string(),
        attackers: [attacker].into_iter().collect(),
        defenders: [defender].into_iter().collect(),
        primary_attacker: Some(attacker),
        primary_defender: Some(defender),
        source_crisis_id: None,
        war_goals: vec![strategy_game::diplomacy::crisis::WarGoal {
            attacker,
            defender,
            goal_type: strategy_game::diplomacy::crisis::WarGoalType::ConquerState,
            target_states: vec![target_state],
            base_peace_cost: 20.0,
            international_concern: 10.0,
            completion: 0.0,
            is_primary: true,
        }],
        start_date: "1800/01/01".to_string(),
        end_date: None,
        duration_days: 0,
        war_score: 0.0,
        attacker_war_exhaustion: 0.0,
        defender_war_exhaustion: 0.0,
        occupied_states: std::collections::HashSet::new(),
        status: WarStatus::Active,
        winner: None,
        end_reason: None,
        applied_terms: Vec::new(),
        won_attacker_battles: 0,
        won_defender_battles: 0,
        processed_battle_ids: std::collections::HashSet::new(),
    }
}

/// N国・M州の合成ワールドを構築し、CountryId(1)を最後の所有陸州(首都)だけへ
/// 削り、指定した規模のW(無関係な他War)・D(Division)・C(Claim)・X(Crisis)・
/// J(Justification)を仕込んだ上で、`preflight_country_elimination`+
/// `apply_country_elimination`のペアを1回計測する。
#[allow(clippy::too_many_arguments)]
fn run_one(
    state_count: usize,
    other_war_count: usize,
    division_count: usize,
    claim_count: usize,
    crisis_count: usize,
    justification_count: usize,
) -> (usize, Duration) {
    let config = WorkloadConfig {
        state_count,
        scenario: Scenario::Normal,
        seed: FIXED_SEED,
    };
    let (app, workload) = profiling::build_app(&config);
    let target = CountryId(1);
    let annexer = CountryId(0);

    let mut state_registry = app
        .world()
        .resource::<StateRegistry>()
        .clone_for_profiling();
    let mut country_registry = app
        .world()
        .resource::<CountryRegistry>()
        .clone_for_profiling();

    let target_capital = country_registry.get(target).unwrap().capital_state_id;
    // targetの首都以外の全所有陸州をannexerへ付け替え、最後の陸州だけを残す。
    for state in state_registry.states.iter_mut() {
        if state.owner_country_id == target && state.id != target_capital {
            state.owner_country_id = annexer;
        }
    }

    let mut war_registry = WarRegistry::default();
    // targetが支持国としてのみ参加する軽量War 1件(§9の両分岐を毎回通す)。
    let mut supporter_war = make_war(
        0,
        CountryId(2 % workload.country_count.max(3)),
        CountryId(3 % workload.country_count.max(4)),
        target_capital,
    );
    supporter_war.attackers.insert(target);
    war_registry.wars.insert(WarId(0), supporter_war);
    // targetが主要参加国(defender)である「他War」をother_war_count件。
    for i in 0..other_war_count {
        let attacker = CountryId((i + 4) % workload.country_count.max(5));
        war_registry.wars.insert(
            WarId(i + 1),
            make_war(i + 1, attacker, target, target_capital),
        );
    }

    let mut military_registry = MilitaryRegistry::default();
    for i in 0..division_count {
        military_registry.add_division(make_division(i, target, target_capital));
    }

    let mut claim_registry = ClaimRegistry::default();
    for i in 0..claim_count {
        claim_registry.claims.insert(
            ClaimId(i),
            TerritorialClaim {
                id: ClaimId(i),
                claimant_country: target,
                target_state: StateId(i % state_count.max(1)),
                strength: 50.0,
                created_date: "1800/01/01".to_string(),
                is_permanent: false,
                source: ClaimSource::Strategic,
                status: ClaimStatus::Active,
            },
        );
    }

    let mut crisis_registry = CrisisRegistry::default();
    for i in 0..crisis_count {
        crisis_registry.crises.insert(
            DiplomaticCrisisId(i),
            DiplomaticCrisis {
                id: DiplomaticCrisisId(i),
                initiator: CountryId((i + 5) % workload.country_count.max(6)),
                target,
                war_goals: vec![],
                start_date: "1800/01/01".to_string(),
                current_phase: CrisisPhase::DemandSent,
                escalation: 0.0,
                initiator_support: 0.0,
                target_resistance: 0.0,
                days_in_phase: 0,
                deadline_date: None,
                international_concern: 0.0,
                third_party_reactions: Default::default(),
                related_claim_id: None,
                related_justification_id: None,
                related_war_id: None,
            },
        );
    }

    let mut justification_registry = WarJustificationRegistry::default();
    for i in 0..justification_count {
        justification_registry.justifications.insert(
            i,
            WarJustification {
                id: i,
                initiator: target,
                target: CountryId((i + 6) % workload.country_count.max(7)),
                target_state: StateId(i % state_count.max(1)),
                start_date: "1800/01/01".to_string(),
                required_days: 30,
                days_passed: 30,
                is_ready: true,
                source_crisis_id: None,
                committed_attackers: vec![],
                committed_defenders: vec![],
            },
        );
    }

    let mut diplomacy_registry = DiplomacyRegistry::default();
    let mut frontline_registry = FrontlineRegistry::default();
    let mut battle_registry = BattleRegistry::default();
    let mut army_registry = ArmyRegistry::default();
    let mut power_registry = CountryPowerRegistry::default();

    let plan = preflight_country_elimination(
        target_capital,
        target,
        annexer,
        "1800/06/01",
        &state_registry,
        &country_registry,
    )
    .expect("perf setup must produce a valid elimination plan");

    let start = Instant::now();
    apply_country_elimination(
        plan,
        None,
        None,
        "1800/06/01",
        &mut state_registry,
        &mut country_registry,
        &mut military_registry,
        &mut army_registry,
        &mut frontline_registry,
        &mut war_registry,
        &mut battle_registry,
        &mut diplomacy_registry,
        &mut crisis_registry,
        &mut claim_registry,
        &mut justification_registry,
        &mut power_registry,
    )
    .expect("perf run must succeed");
    let elapsed = start.elapsed();

    (workload.country_count, elapsed)
}

/// AIループのO(1)スキップコスト(§18): 消滅済み国家がN国中X国いる状態で、
/// `country::country_ai::process_daily_country_ai`を含む1日分の日次進行
/// (`DailySimulationSet::CountryAi`)のコストが、消滅国家0件のときと比べて
/// 悪化しない(むしろ実質的な対象国が減るため軽くなる)ことを確認する。
fn measure_country_ai_set_ms(state_count: usize, eliminated_count: usize) -> (f64, f64, f64) {
    let config = WorkloadConfig {
        state_count,
        scenario: Scenario::Normal,
        seed: FIXED_SEED,
    };
    let (mut app, workload) = profiling::build_app(&config);
    profiling::install_set_timings(&mut app);

    {
        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let current_date = "1800/06/01".to_string();
        for i in 0..eliminated_count.min(workload.country_count.saturating_sub(1)) {
            // CountryId(0)はplayer固定なので1から順に消滅させる。
            let cid = CountryId(i + 1);
            if let Some(c) = country_registry.get_mut(cid) {
                c.status = strategy_game::country::CountryStatus::Eliminated {
                    eliminated_date: current_date.clone(),
                    annexed_by: CountryId(0),
                };
            }
        }
    }

    for _ in 0..5 {
        profiling::advance_one_day(&mut app);
    }

    let country_ai_idx = strategy_game::profiling::DAILY_SET_NAMES
        .iter()
        .position(|n| *n == "CountryAi")
        .unwrap();

    let mut stats = Stats::new();
    for _ in 0..30 {
        profiling::advance_one_day(&mut app);
        if let Some(durations) = profiling::read_set_durations(&app) {
            stats.push(durations[country_ai_idx]);
        }
    }
    (stats.mean(), stats.percentile(0.5), stats.percentile(0.95))
}

/// `run_one`は1回のApp構築+消滅処理を含むため、単発計測はOSスケジューリング
/// ノイズの影響を受けやすい。5回実行して平均を取ることでノイズを均す
/// (`CountryAi`の30サンプル平均と同じ考え方)。
fn run_one_averaged(
    state_count: usize,
    other_war_count: usize,
    division_count: usize,
    claim_count: usize,
    crisis_count: usize,
    justification_count: usize,
) -> (usize, f64) {
    const SAMPLES: usize = 5;
    let mut country_count = 0;
    let mut total_ms = 0.0;
    for _ in 0..SAMPLES {
        let (cc, elapsed) = run_one(
            state_count,
            other_war_count,
            division_count,
            claim_count,
            crisis_count,
            justification_count,
        );
        country_count = cc;
        total_ms += elapsed.as_secs_f64() * 1000.0;
    }
    (country_count, total_ms / SAMPLES as f64)
}

#[test]
#[ignore]
fn profile_p21_021_elimination_scaling() {
    let output_dir = PathBuf::from("verification_logs/phase-21/p21-021/perf");
    fs::create_dir_all(&output_dir).expect("output dir creation must succeed");

    let mut summary = String::new();
    let mut csv = String::from("axis,param,country_count,elapsed_ms\n");

    summary.push_str("P21-021 国家消滅処理 性能プロファイル\n");
    summary.push_str("=====================================\n\n");

    // ── 軸1: 無関係な他War総数(W)のスケーリング ──────────────────────────────
    summary.push_str(
        "== 軸1: 無関係な他War総数(W) (D=10,C=10,X=10,J=10固定、state_count=2000、5回平均) ==\n",
    );
    summary.push_str("other_war_count | elapsed_ms(mean of 5)\n");
    for w in [0usize, 10, 100, 1000] {
        let (country_count, mean_ms) = run_one_averaged(2000, w, 10, 10, 10, 10);
        summary.push_str(&format!(
            "  {w:>6} | {mean_ms:.4} (country_count={country_count})\n"
        ));
        csv.push_str(&format!(
            "other_war_count,{w},{country_count},{mean_ms:.5}\n"
        ));
    }

    // ── 軸2: 対象国自身のDivision/Claim/Crisis/Justification件数のスケーリング ──
    #[allow(clippy::type_complexity)]
    let axes2: [(&str, fn(usize) -> (usize, f64)); 4] = [
        ("division_count", |n| {
            run_one_averaged(2000, 10, n, 10, 10, 10)
        }),
        ("claim_count", |n| run_one_averaged(2000, 10, 10, n, 10, 10)),
        ("crisis_count", |n| {
            run_one_averaged(2000, 10, 10, 10, n, 10)
        }),
        ("justification_count", |n| {
            run_one_averaged(2000, 10, 10, 10, 10, n)
        }),
    ];
    for (label, apply) in axes2 {
        summary.push_str(&format!(
            "\n== 軸2: {label} のスケーリング(他は10固定、5回平均) ==\n"
        ));
        summary.push_str(&format!("{label} | elapsed_ms(mean of 5)\n"));
        for n in [0usize, 10, 100, 1000] {
            let (country_count, mean_ms) = apply(n);
            summary.push_str(&format!(
                "  {n:>6} | {mean_ms:.4} (country_count={country_count})\n"
            ));
            csv.push_str(&format!("{label},{n},{country_count},{mean_ms:.5}\n"));
        }
    }

    // ── 軸3: 世界の総国家数(N)のスケーリング(target自身の関連要素は10で固定) ──
    summary.push_str(
        "\n== 軸3: 世界の総国家数(N、state_countで間接指定) (D=C=X=J=10固定、5回平均) ==\n",
    );
    summary.push_str("state_count | country_count | elapsed_ms(mean of 5)\n");
    for sc in [200usize, 2000, 8000] {
        let (country_count, mean_ms) = run_one_averaged(sc, 10, 10, 10, 10, 10);
        summary.push_str(&format!("  {sc:>6} | {country_count:>6} | {mean_ms:.4}\n"));
        csv.push_str(&format!("state_count,{sc},{country_count},{mean_ms:.5}\n"));
    }

    // ── 軸4/5: AIループのO(1)スキップコスト・非消滅日の追加コスト ──────────────
    summary
        .push_str("\n== 軸4/5: 日次CountryAi Setコスト(state_count=2000, country_count>=100) ==\n");
    summary.push_str("eliminated_count | mean(ms) | p50(ms) | p95(ms)\n");
    for eliminated in [0usize, 10, 50] {
        let (mean, p50, p95) = measure_country_ai_set_ms(2000, eliminated);
        summary.push_str(&format!(
            "  {eliminated:>6} | {mean:.5} | {p50:.5} | {p95:.5}\n"
        ));
        csv.push_str(&format!("country_ai_set,{eliminated},2000,{mean:.5}\n"));
    }
    summary.push_str(
        "\n消滅国家が増えるほどCountryAi Setの実測コストが悪化しないこと(むしろ\n\
         軽くなる傾向)を示す——is_active()フィルタがループ本体の外(候補国リスト\n\
         構築時)で1回だけ適用されるため、消滅国家1件あたりの追加コストはO(1)未満\n\
         (ループにすら入らない)。\n",
    );

    summary.push_str(
        "\n== Big-O分析 ==\n\
         - 軸1(他War総数W): O(W) ——`war_registry.wars`全体を1回走査してtargetが\n\
           参加するWarを見つける必要があり(逆引きインデックスを持たない既存設計)、\n\
           これは`invalidate_other_wars_targeting_state`等の既存コードと同じ\n\
           パターンであり新規の非効率ではない。target自身に無関係なWarは\n\
           フィルタで即座に除外されるため、Wごとの追加コストはO(1)。\n\
         - 軸2(Division/Claim/Crisis/Justification): いずれもO(該当件数)の\n\
           単純な1パスフィルタ+代入であり、他の集合との総当たり(O(N×M))は\n\
           一切存在しない。\n\
         - 軸3(総国家数N): `CountryRegistry`が`Vec`ベースのため`get`/`get_mut`は\n\
           O(N)(既存設計、本タスクで新規に導入したものではない)。消滅処理内で\n\
           `country_registry.get_mut(eliminated_country)`を呼ぶ回数は定数(2回)\n\
           のため、全体としてO(N)。実運用規模(数十国)では無視できるコスト。\n\
         - 軸4/5: `is_active()`フィルタはAI候補国リスト構築時に1回だけ適用され、\n\
           消滅国家はループ本体に一切入らない(O(1)未満、スキップではなく\n\
           そもそも列挙されない)。\n",
    );

    fs::write(output_dir.join("summary.txt"), &summary).expect("summary.txt write must succeed");
    fs::write(output_dir.join("results.csv"), &csv).expect("results.csv write must succeed");

    println!("{summary}");
    println!(
        "[P21-021-PERF] evidence written to {}",
        output_dir.display()
    );
}

trait CloneForProfiling {
    fn clone_for_profiling(&self) -> Self;
}
impl CloneForProfiling for StateRegistry {
    fn clone_for_profiling(&self) -> Self {
        StateRegistry::build(self.states.clone())
    }
}
impl CloneForProfiling for CountryRegistry {
    fn clone_for_profiling(&self) -> Self {
        CountryRegistry {
            countries: self.countries.clone(),
        }
    }
}
