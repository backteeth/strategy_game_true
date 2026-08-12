//! P20-008: プロファイリング用ワールド生成の正しさを検証する回帰テスト。
//!
//! `src/bin/profile_1000_states.rs` が使用するものと全く同じ
//! `strategy_game::profiling` の生成・進行ロジックを、小規模(高速に実行できる)
//! State数で呼び出すことで、計測経路と検証経路の乖離を防ぐ。
//! `cargo test` を極端に遅くしないよう、ここでは数十〜200州程度に留める。

use strategy_game::profiling::{
    self, Scenario, WorkloadConfig, country_count_for, snapshot_runtime_counts,
    validate_world_sanity,
};

const TEST_SEED: u64 = 0x00C0_FFEE_1234_5678;

#[test]
fn generated_state_count_matches_requested() {
    let config = WorkloadConfig {
        state_count: 137,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };
    let (app, counts) = profiling::build_app(&config);

    assert_eq!(counts.state_count, 137, "生成State数が指定値と一致すること");

    let actual_states = app
        .world()
        .resource::<strategy_game::state::data::StateRegistry>()
        .states
        .len();
    assert_eq!(
        actual_states, 137,
        "StateRegistryの実際のState数が指定値と一致すること"
    );

    assert_eq!(
        counts.country_count,
        country_count_for(137),
        "国家数が定義済み比率から算出した値と一致すること"
    );
}

#[test]
fn state_country_associations_are_valid() {
    let config = WorkloadConfig {
        state_count: 240,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };
    let (app, _counts) = profiling::build_app(&config);

    validate_world_sanity(&app).expect("生成直後のワールドは健全であること");

    let world = app.world();
    let state_registry = world.resource::<strategy_game::state::data::StateRegistry>();
    let country_registry = world.resource::<strategy_game::country::CountryRegistry>();

    assert!(
        !country_registry.countries.is_empty(),
        "国家が1つ以上生成されること"
    );

    for state in &state_registry.states {
        assert!(
            country_registry.get(state.owner_country_id).is_some(),
            "State {:?} の所有国 {:?} が実在すること",
            state.id,
            state.owner_country_id
        );
    }

    for country in &country_registry.countries {
        assert!(
            state_registry.get(country.capital_state_id).is_some(),
            "Country {:?} の首都 {:?} が実在すること",
            country.id,
            country.capital_state_id
        );
    }
}

#[test]
fn daily_tick_advances_requested_number_of_days() {
    let config = WorkloadConfig {
        state_count: 60,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };
    let (mut app, _counts) = profiling::build_app(&config);

    let initial_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .clone();

    for _ in 0..10 {
        profiling::advance_one_day(&mut app);
    }

    let final_date = app
        .world()
        .resource::<strategy_game::app::time::GameDate>()
        .clone();

    assert_ne!(initial_date, final_date, "日次tickにより日付が進行すること");
}

/// 通常シナリオで経済・研究・外交が実際に変化を起こすことを確認する。
/// 経済・研究の重い処理は月次(30日毎)に発生するため、1ヶ月+バッファを進める。
///
/// 注: 本プロトタイプの現行実装には人口"成長"システムが存在しない
/// (`update_state_population_and_employment` は `population` フィールド自体は
/// 変更せず、`employed_workforce`/`unemployed_workforce` のみを更新する)。
/// そのため人口の"総数"ではなく、月次処理で実際に変化する
/// 就業者数(employed_workforce)合計を経済活動の指標として使用する。
#[test]
fn normal_scenario_produces_real_economic_and_research_activity() {
    let config = WorkloadConfig {
        state_count: 100,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };
    let (mut app, _counts) = profiling::build_app(&config);

    let treasury_before = profiling::total_treasury(&app);
    let employed_before = total_employed_workforce(&app);

    for _ in 0..40 {
        profiling::advance_one_day(&mut app);
    }

    validate_world_sanity(&app).expect("40日進行後もワールドは健全であること");

    let treasury_after = profiling::total_treasury(&app);
    let employed_after = total_employed_workforce(&app);

    assert_ne!(
        treasury_before, treasury_after,
        "40日間(月次経済処理を含む)で国家国庫合計が変化すること"
    );
    assert_ne!(
        employed_before, employed_after,
        "40日間(月次人口・雇用処理を含む)でState就業者数合計が変化すること"
    );
}

fn total_employed_workforce(app: &bevy::app::App) -> u64 {
    app.world()
        .resource::<strategy_game::state::data::StateRegistry>()
        .states
        .iter()
        .map(|s| s.employed_workforce)
        .sum()
}

/// 高負荷シナリオで戦争・軍事AI・前線処理が実際に実行されることを確認する。
#[test]
fn high_load_scenario_produces_real_war_and_military_activity() {
    let config = WorkloadConfig {
        state_count: 100,
        scenario: Scenario::HighLoad,
        seed: TEST_SEED,
    };
    let (mut app, counts) = profiling::build_app(&config);

    assert!(
        counts.initial_war_count > 0,
        "高負荷シナリオでは初期状態から1件以上の戦争が生成されること"
    );

    let before = snapshot_runtime_counts(&app);
    assert!(before.division_count > 0, "軍隊が1件以上生成されること");

    for _ in 0..15 {
        profiling::advance_one_day(&mut app);
    }

    validate_world_sanity(&app).expect("15日進行後もワールドは健全であること");

    let after = snapshot_runtime_counts(&app);
    assert!(
        after.frontline_count > 0,
        "戦争が存在する場合、日次WarPreparation処理により前線が1件以上生成されること"
    );
}

/// 同一Seed・同一入力からは常に同一の論理結果が得られることを確認する(決定論)。
#[test]
fn same_seed_produces_deterministic_results() {
    let config = WorkloadConfig {
        state_count: 150,
        scenario: Scenario::HighLoad,
        seed: TEST_SEED,
    };

    let (mut app_a, counts_a) = profiling::build_app(&config);
    let (mut app_b, counts_b) = profiling::build_app(&config);

    assert_eq!(counts_a.state_count, counts_b.state_count);
    assert_eq!(counts_a.country_count, counts_b.country_count);
    assert_eq!(counts_a.initial_division_count, counts_b.initial_division_count);
    assert_eq!(counts_a.initial_war_count, counts_b.initial_war_count);

    for _ in 0..20 {
        profiling::advance_one_day(&mut app_a);
        profiling::advance_one_day(&mut app_b);
    }

    assert_eq!(
        profiling::total_treasury(&app_a),
        profiling::total_treasury(&app_b),
        "同一Seed・同一tick数であれば国庫合計は完全一致すること"
    );
    assert_eq!(
        profiling::total_population(&app_a),
        profiling::total_population(&app_b),
        "同一Seed・同一tick数であれば人口合計は完全一致すること"
    );

    let runtime_a = snapshot_runtime_counts(&app_a);
    let runtime_b = snapshot_runtime_counts(&app_b);
    assert_eq!(runtime_a.division_count, runtime_b.division_count);
    assert_eq!(runtime_a.active_war_count, runtime_b.active_war_count);
    assert_eq!(runtime_a.frontline_count, runtime_b.frontline_count);
    assert_eq!(runtime_a.battle_count, runtime_b.battle_count);
}

/// 大きめの規模でも数日程度なら健全性が保たれることを確認する
/// (1000州規模の完全な代表計測は `profile_1000_states` バイナリ側で行う。
/// ここでは `cargo test` の実行時間を抑えるため200州・5日に留める)。
#[test]
fn larger_scale_smoke_test_stays_sane() {
    let config = WorkloadConfig {
        state_count: 200,
        scenario: Scenario::HighLoad,
        seed: TEST_SEED,
    };
    let (mut app, counts) = profiling::build_app(&config);
    assert_eq!(counts.state_count, 200);

    for _ in 0..5 {
        profiling::advance_one_day(&mut app);
    }

    validate_world_sanity(&app).expect("200州規模でも5日間の進行後にワールドが健全であること");
}

// ─────────────────────────────────────────────────────────────────────────
// P20-008追補: 研究・外交の実変化検証
//
// 経済(国庫・就業者数)は既存テストで実変化を確認済みだが、研究・外交は
// デフォルト状態のままでは実データが変化しない(実ソース確認済み、下記コメント参照)。
// そのため、本番システムが実際に処理対象とする「有効な状態」を明示的に構築した上で、
// 本番のDailySimulationSet経路(`handle_monthly_research` / `handle_daily_diplomacy`)
// が実データを変化させることを検証する。日付はテストコードから直接書き換えず、
// 常に `profiling::advance_one_day` (本番と同じ `GameDate.accumulator` 経由) を使う。
// ─────────────────────────────────────────────────────────────────────────

use strategy_game::common::CountryId;
use strategy_game::country::CountryRegistry;
use strategy_game::diplomacy::data::{
    ActiveDiplomaticActivity, DiplomacyRegistry, DiplomaticActivityType, DiplomaticPairKey,
};
use strategy_game::research::allocation::{InProgressTech, ResearchAllocation};
use strategy_game::research::data::TechnologyRegistry;

/// 研究データ自体(進捗または技術完了)が本番の月次研究処理で実際に変化することを検証する。
///
/// 注: `CountryData::default()` の `research_state.in_progress` は空であり、
/// `handle_monthly_research` は空の in_progress に対しては何もしない
/// (`handle_npc_auto_research` がプレイヤー以外の空フィールドへ新規技術を割り当てるのは
/// 月次処理の"後"であるため、初回月では進捗が発生しない)。
/// そのため本テストでは、実データ(`technologies.ron`から読み込まれた実際の技術定義)を用いて
/// 研究中の技術をあらかじめ設定し、本番の月次処理が実際に進捗させることを検証する。
#[test]
fn research_data_advances_via_real_monthly_research_system() {
    let config = WorkloadConfig {
        state_count: 60,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };

    // 決定論検証のため同一設定で2つ独立に構築する
    let (mut app_a, _) = profiling::build_app(&config);
    let (mut app_b, _) = profiling::build_app(&config);

    for app in [&mut app_a, &mut app_b] {
        let (tech_id, tech_cost, field) = {
            let tech_registry = app.world().resource::<TechnologyRegistry>();
            let id = tech_registry
                .sorted_ids
                .first()
                .cloned()
                .expect("technologies.ron に技術が1件以上存在すること(本番データ)");
            let def = tech_registry
                .get(&id)
                .expect("選択した技術IDが定義に存在すること");
            (id, def.cost, def.field)
        };

        let mut country_registry = app.world_mut().resource_mut::<CountryRegistry>();
        let country = country_registry
            .countries
            .iter_mut()
            .find(|c| c.id == CountryId(1))
            .expect("Country(1) が生成されていること(AI国家、プレイヤーはCountry(0))");

        // 全分野に配分することで、選択した技術の分野に関わらず月次ポイントが発生するようにする
        country.research_state.allocation = ResearchAllocation {
            science: 0.4,
            magic: 0.3,
            military: 0.2,
            fusion: 0.1,
        };
        country.science_research_capacity = 50.0;
        country.magic_research_capacity = 50.0;
        country.research_state.in_progress.insert(
            field,
            InProgressTech {
                tech_id: tech_id.clone(),
                progress: 0.0,
                cost: tech_cost,
            },
        );
    }

    // 月次研究処理(30日毎)を確実に跨ぐため35日進める。日付はGameDateのaccumulator経由でのみ進める。
    for _ in 0..35 {
        profiling::advance_one_day(&mut app_a);
        profiling::advance_one_day(&mut app_b);
    }

    validate_world_sanity(&app_a).expect("35日進行後もワールドは健全であること(app_a)");
    validate_world_sanity(&app_b).expect("35日進行後もワールドは健全であること(app_b)");

    let read_progress = |app: &bevy::app::App| -> (bool, f64, bool) {
        let country_registry = app.world().resource::<CountryRegistry>();
        let country = country_registry.get(CountryId(1)).unwrap();
        // in_progressのいずれかの分野で進捗があるか、または完了技術に含まれるかを確認
        let any_progress = country
            .research_state
            .in_progress
            .values()
            .any(|p| p.progress > 0.0);
        let sum_progress: f64 = country
            .research_state
            .in_progress
            .values()
            .map(|p| p.progress)
            .sum();
        let completed = !country.research_state.completed_technologies.is_empty();
        (any_progress, sum_progress, completed)
    };

    let (any_progress_a, sum_progress_a, completed_a) = read_progress(&app_a);
    let (_any_progress_b, sum_progress_b, completed_b) = read_progress(&app_b);

    assert!(
        any_progress_a || completed_a,
        "35日間(月次研究処理を含む)でCountry(1)の研究データ自体(進捗または技術完了)が変化すること"
    );

    // 決定論: 同一Seed・同一手順であれば研究進捗の実測値も完全一致する
    assert_eq!(
        sum_progress_a, sum_progress_b,
        "同一Seedであれば研究進捗の合計値は完全一致すること"
    );
    assert_eq!(
        completed_a, completed_b,
        "同一Seedであれば技術完了の有無も完全一致すること"
    );
}

/// 外交関係データが本番の日次Diplomacy処理で実際に変化することを検証する。
///
/// 注: 合成ワールドの`DiplomaticRelation`は全ペースが`DiplomaticRelation::default()`
/// (opinion=0, active_activity=None, cooldowns空)であり、`handle_daily_diplomacy`は
/// `active_activity`が`Some`の関係のみopinionを変化させる(実ソース確認済み、
/// `src/diplomacy/update.rs::handle_daily_diplomacy`参照)。そのため本テストでは、
/// 本番の`ActiveDiplomaticActivity`構造体を用いて実際に進行中の外交活動を構築し、
/// 本番システムがそれを処理して実データを変化させることを検証する。
#[test]
fn diplomatic_relation_advances_via_real_daily_diplomacy_system() {
    let config = WorkloadConfig {
        state_count: 60,
        scenario: Scenario::Normal,
        seed: TEST_SEED,
    };
    let (mut app, _counts) = profiling::build_app(&config);

    let key = DiplomaticPairKey::new(CountryId(0), CountryId(1))
        .expect("Country(0)とCountry(1)は異なる国であること");

    {
        let mut diplomacy_registry = app.world_mut().resource_mut::<DiplomacyRegistry>();
        let relation = diplomacy_registry
            .relations
            .get_mut(&key)
            .expect("Country(0)-Country(1)間の外交関係が生成されていること(全ペア生成済み)");
        relation.opinion = 0.0;
        relation.active_activity = Some(ActiveDiplomaticActivity {
            activity_type: DiplomaticActivityType::ImproveRelations,
            initiator: CountryId(0),
            target: CountryId(1),
            days_remaining: 10,
            daily_opinion_change: 2.0,
        });
    }

    for _ in 0..3 {
        profiling::advance_one_day(&mut app);
    }

    validate_world_sanity(&app).expect("3日進行後もワールドは健全であること");

    let diplomacy_registry = app.world().resource::<DiplomacyRegistry>();
    let relation = diplomacy_registry.relations.get(&key).unwrap();

    assert_eq!(
        relation.opinion, 6.0,
        "本番の日次Diplomacy処理により、進行中の外交活動(1日+2.0)を3日分反映してopinionが6.0になること"
    );
    assert_eq!(
        relation.active_activity.as_ref().map(|a| a.days_remaining),
        Some(7),
        "本番の日次Diplomacy処理により、活動の残り日数が3日分減少すること(10→7)"
    );
}
