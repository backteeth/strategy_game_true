//! P23-001: 州人口の月次成長と、それに連動した`available_manpower`の月次回復。
//!
//! 現行実装には`StateData.population`を増加させる経路が一切存在せず(`update_state_
//! population_and_employment`は読むだけ)、`CountryData.available_manpower`も募兵消費
//! ・キャンセル返還のみで時間経過による回復が無かった。1800〜1950年という150年規模の
//! 対象期間に対し、人口・人的資源が時間で全く動かないのは中核ループの欠落である
//! (Prototype v0.2基準文書§Phase23候補の根拠)。
//!
//! 複雑な人口動態(年齢構成・出生死亡・移民・戦争被害・疫病等)は導入しない。浮動小数点も
//! 使わず、全て整数のbasis point(1万分率)固定小数点で計算し、決定論を保つ。

use crate::app::time::MonthChangedMessage;
use crate::common::CountryId;
use crate::country::CountryRegistry;
use crate::economy::state_access::StateEconomyAccess;
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use std::collections::HashMap;

/// 固定小数点の分母(1万分率、basis points)。
pub const BASIS_POINTS_DENOMINATOR: u64 = 10_000;

/// 月次人口成長率(basis points)。5/10000 = 0.05%/月 ≒ 年率0.6%。
/// 150年(1800か月)複利で概ね2.46倍(`e^(0.0005*1800)`)——ユーザー指定の
/// 「開始時の2〜3倍に収まる水準」を満たす。実データ(`assets/data/states.ron`)の
/// 最小州人口は25,000であり、この率でも月あたり12(floor)の非ゼロ成長を得る。
pub const MONTHLY_POPULATION_GROWTH_RATE_BP: u64 = 5;

/// 人的資源基準上限のbasis points(所有州人口合計の10%)。
pub const MANPOWER_CAP_RATIO_BP: u64 = 1_000;

/// 人的資源回復期間(月数)。空の人的資源が基準上限まで概ね10年で回復する速度。
pub const MANPOWER_RECOVERY_PERIOD_MONTHS: u64 = 120;

/// 1州分の人口を1か月ぶん成長させる純粋関数。
///
/// 端数は切り捨てる(繰越フィールドは今回のスコープでは導入しない)。人口0の州は
/// `0 * rate / denom == 0`により自然に0のまま(特別扱い不要)。小人口州のための
/// 最低成長数の強制は行わない(極小州だけが異常な実効成長率になるため)。
/// `saturating_mul`/`saturating_add`によりオーバーフロー時もwrapしない
/// (実データの規模では発生し得ないが、防御的に扱う)。
pub fn grow_state_population(population: u64) -> u64 {
    let growth =
        population.saturating_mul(MONTHLY_POPULATION_GROWTH_RATE_BP) / BASIS_POINTS_DENOMINATOR;
    population.saturating_add(growth)
}

/// `StateRegistry`内の全州人口を1か月ぶん成長させる(O(州数)、1回の走査)。
pub fn grow_all_state_populations(state_registry: &mut StateRegistry) {
    // P32-008: 成長の中核は`grow_populations`へ集約し、旧`StateData`も新ステートも同じ関数を通る。
    let mut refs: Vec<&mut crate::state::data::StateData> = state_registry.states.iter_mut().collect();
    grow_populations(&mut refs);
}

/// 地域の人口を1か月ぶん成長させる共通実装。各地域を**ちょうど1回**だけ更新し(所属プロヴィンス数・
/// 配列の順序・呼び出し回数以外の要因で増幅しない)、式は`grow_state_population`のまま。
pub fn grow_populations<S: StateEconomyAccess>(states: &mut [&mut S]) {
    for s in states.iter_mut() {
        let grown = grow_state_population(s.population());
        s.set_population(grown);
    }
}

/// 国家の人的資源の月次回復(所有人口から基準上限を求め、`recover_manpower`で1か月ぶん回復)。
/// 旧`handle_monthly_population_growth`と新ステートの検証経路が共通で使う。国家の値は
/// 国家ごとに1回だけ更新し、各ステートから重複して更新しない。
pub fn recover_country_manpower(available_manpower: u64, owned_population: u64) -> u64 {
    recover_manpower(available_manpower, compute_manpower_cap(owned_population))
}

/// 州の法的所有国(`owner_country_id`、占領による実効支配ではない)ごとに人口を集計する
/// (O(州数)、1回の走査)。`country::power::compute_state_aggregates`と同じ規約
/// (所有国基準)だが、あちらは表示用ランキングのため`f64`を使う一方、こちらは
/// `available_manpower`という実際にゲームプレイへ影響する値の算出に使うため、決定論を
/// 保つ整数(`u64`)のまま合算する。
///
/// 加算は可換なため、`state_registry.states`の走査順(=挿入順)やHashMapの内部走査順に
/// 結果が依存しない(合計値は集合として一意に定まる)。
pub fn compute_population_by_owner(state_registry: &StateRegistry) -> HashMap<CountryId, u64> {
    let mut totals: HashMap<CountryId, u64> = HashMap::new();
    for state in &state_registry.states {
        *totals.entry(state.owner_country_id).or_insert(0) += state.population;
    }
    totals
}

/// 所有人口から人的資源の基準上限を算出する純粋関数。
pub fn compute_manpower_cap(population: u64) -> u64 {
    population.saturating_mul(MANPOWER_CAP_RATIO_BP) / BASIS_POINTS_DENOMINATOR
}

/// `available_manpower`を基準上限へ向けて1か月ぶん回復させる純粋関数。
///
/// - `cap == 0`(所有州なし、または人口0)、または既に`current >= cap`
///   (上限到達済み、または州喪失で上限が現在値を下回った直後)の場合は現状維持する
///   (削除せず、回復もしない)。
/// - それ以外は`cap / MANPOWER_RECOVERY_PERIOD_MONTHS`(基準上限の1/120)だけ回復するが、
///   整数丸めで0になる場合は最低1回復し(`cap > 0`かつ`current < cap`の場合に限る)、
///   不足分(`cap - current`)を超えて回復することはない。
pub fn recover_manpower(current: u64, cap: u64) -> u64 {
    if cap == 0 || current >= cap {
        return current;
    }
    let deficit = cap - current;
    let raw_recovery = cap / MANPOWER_RECOVERY_PERIOD_MONTHS;
    let recovery = raw_recovery.max(1).min(deficit);
    current + recovery
}

/// P23-001の月次System本体。`MonthChangedMessage`1件につき正確に1回、
/// 州人口成長→所有国別人口集計→Active国家の人的資源回復、の順で実行する。
/// 高速なゲーム速度で1フレームに複数月が経過した場合(`app::time::advance_game_date`)、
/// メッセージの件数ぶんだけ繰り返す(1回の月替わりで複数回・0回にはならない)。
///
/// 計算量: 州走査2回(成長・集計)+ 国家走査1回(回復)= O(州数 + 国家数)。
/// 国家ごとに全州を再走査するO(国家数×州数)は行わない。
pub fn handle_monthly_population_growth(
    mut month_events: MessageReader<MonthChangedMessage>,
    mut state_registry: ResMut<StateRegistry>,
    mut country_registry: ResMut<CountryRegistry>,
) {
    for _event in month_events.read() {
        grow_all_state_populations(&mut state_registry);
        let population_by_owner = compute_population_by_owner(&state_registry);

        for country in country_registry.countries.iter_mut() {
            if !country.is_active() {
                continue;
            }
            let population = population_by_owner.get(&country.id).copied().unwrap_or(0);
            country.available_manpower =
                recover_country_manpower(country.available_manpower, population);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{CountryData, CountryStatus};
    use crate::state::data::StateData;
    use bevy::state::app::StatesPlugin;

    fn state(id: usize, owner: usize, population: u64) -> StateData {
        StateData {
            id: crate::common::StateId(id),
            owner_country_id: CountryId(owner),
            population,
            ..StateData::default()
        }
    }

    fn country(id: usize, available_manpower: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            available_manpower,
            ..CountryData::default()
        }
    }

    fn eliminated_country(id: usize, annexed_by: usize, available_manpower: u64) -> CountryData {
        CountryData {
            id: CountryId(id),
            available_manpower,
            status: CountryStatus::Eliminated {
                eliminated_date: "1850/01/01".to_string(),
                annexed_by: CountryId(annexed_by),
            },
            ..CountryData::default()
        }
    }

    // ─── 人口成長: 純粋関数 ───────────────────────────────────────────────────

    #[test]
    fn grow_state_population_zero_stays_zero() {
        assert_eq!(grow_state_population(0), 0);
    }

    #[test]
    fn grow_state_population_rounds_down_for_a_small_population() {
        // 25,000(実データの最小州人口)* 5 / 10,000 = 12.5 -> floor 12。
        assert_eq!(grow_state_population(25_000), 25_012);
        // 1,999 * 5 / 10,000 = 0.9995 -> floor 0(最低1人の強制はしない)。
        assert_eq!(grow_state_population(1_999), 1_999);
        // 2,000 * 5 / 10,000 = 1.0 -> ちょうど1。
        assert_eq!(grow_state_population(2_000), 2_001);
    }

    #[test]
    fn grow_state_population_does_not_overflow_for_a_very_large_population() {
        let grown = grow_state_population(u64::MAX);
        assert!(
            grown >= u64::MAX / 2,
            "must not silently wrap to a tiny value"
        );
    }

    #[test]
    fn compute_population_by_owner_sums_multiple_states_and_does_not_cross_owners() {
        let registry = StateRegistry::build(vec![
            state(0, 1, 100_000),
            state(1, 1, 200_000),
            state(2, 2, 50_000),
        ]);
        let totals = compute_population_by_owner(&registry);
        assert_eq!(totals.get(&CountryId(1)).copied(), Some(300_000));
        assert_eq!(totals.get(&CountryId(2)).copied(), Some(50_000));
        assert_eq!(
            totals.len(),
            2,
            "must not invent totals for countries that own no states"
        );
    }

    #[test]
    fn compute_population_by_owner_is_independent_of_insertion_order() {
        let forward = StateRegistry::build(vec![
            state(0, 1, 111_111),
            state(1, 2, 222_222),
            state(2, 1, 333_333),
        ]);
        let reversed = StateRegistry::build(vec![
            state(2, 1, 333_333),
            state(1, 2, 222_222),
            state(0, 1, 111_111),
        ]);
        assert_eq!(
            compute_population_by_owner(&forward),
            compute_population_by_owner(&reversed),
            "summing owned population must not depend on Vec/HashMap iteration order"
        );
    }

    // ─── 人的資源: 純粋関数 ───────────────────────────────────────────────────

    #[test]
    fn compute_manpower_cap_is_ten_percent_of_population() {
        assert_eq!(compute_manpower_cap(1_000_000), 100_000);
        assert_eq!(compute_manpower_cap(0), 0);
    }

    #[test]
    fn recover_manpower_never_exceeds_the_cap() {
        // cap=1200, recovery=1200/120=10。deficitが10未満のときは頭打ちでcapちょうど。
        assert_eq!(recover_manpower(1_195, 1_200), 1_200);
    }

    #[test]
    fn recover_manpower_does_nothing_once_the_cap_is_already_reached() {
        assert_eq!(recover_manpower(1_200, 1_200), 1_200);
    }

    #[test]
    fn recover_manpower_does_not_delete_a_surplus_when_the_cap_drops_below_current() {
        // 州喪失でcapが現在値を下回っても、既存の未使用人的資源を削除しない。
        assert_eq!(recover_manpower(5_000, 1_000), 5_000);
    }

    #[test]
    fn recover_manpower_applies_a_minimum_of_one_when_rounding_would_yield_zero() {
        // cap=50, 50/120=0(floor) -> 最低1回復。
        assert_eq!(recover_manpower(0, 50), 1);
    }

    #[test]
    fn recover_manpower_does_nothing_for_a_zero_cap() {
        assert_eq!(recover_manpower(0, 0), 0);
        assert_eq!(recover_manpower(500, 0), 500);
    }

    // ─── System挙動 ─────────────────────────────────────────────────────────

    fn build_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatesPlugin);
        app.init_state::<crate::app::game_state::GameState>();
        app.insert_state(crate::app::game_state::GameState::Playing);
        app.add_message::<MonthChangedMessage>();
        app.add_systems(Update, handle_monthly_population_growth);
        app
    }

    fn send_month_changed(app: &mut App) {
        let mut writer: bevy::ecs::system::SystemState<MessageWriter<MonthChangedMessage>> =
            bevy::ecs::system::SystemState::new(app.world_mut());
        writer
            .get_mut(app.world_mut())
            .expect("writer")
            .write(MonthChangedMessage {
                year: 1800,
                month: 2,
            });
        writer.apply(app.world_mut());
    }

    #[test]
    fn one_month_changed_message_grows_population_by_exactly_one_month() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_000_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population,
            grow_state_population(1_000_000)
        );
    }

    #[test]
    fn no_month_changed_message_leaves_population_and_manpower_unchanged() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_000_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0)],
        });

        app.update();
        app.update();

        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population,
            1_000_000
        );
        assert_eq!(
            app.world()
                .resource::<CountryRegistry>()
                .get(CountryId(1))
                .unwrap()
                .available_manpower,
            0
        );
    }

    #[test]
    fn two_month_changed_messages_in_one_frame_apply_growth_exactly_twice() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_000_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0)],
        });

        {
            let mut writer: bevy::ecs::system::SystemState<MessageWriter<MonthChangedMessage>> =
                bevy::ecs::system::SystemState::new(app.world_mut());
            let mut w = writer.get_mut(app.world_mut()).expect("writer");
            w.write(MonthChangedMessage {
                year: 1800,
                month: 2,
            });
            w.write(MonthChangedMessage {
                year: 1800,
                month: 3,
            });
            writer.apply(app.world_mut());
        }
        app.update();

        let expected = grow_state_population(grow_state_population(1_000_000));
        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population,
            expected,
            "two MonthChangedMessages queued in the same frame must apply growth exactly twice, \
             matching a fast game-speed month-boundary crossing"
        );
    }

    #[test]
    fn state_population_keeps_growing_after_its_owner_country_is_eliminated() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 2, 500_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![eliminated_country(1, 2, 0), country(2, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        // 州は現所有国(2、Active)のものであり、消滅したのは別国(1)。州人口自体は
        // 「所有国の生死」とは無関係に成長し続ける(住民は所有国の消滅で消えない)。
        assert_eq!(
            app.world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population,
            grow_state_population(500_000)
        );
    }

    #[test]
    fn active_country_manpower_recovers_monthly_toward_its_owned_population_cap() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_200_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        // 成長後人口: 1,200,000 + 600 = 1,200,600。cap = 120,060。recovery = 120,060/120 = 1,000。
        let grown = grow_state_population(1_200_000);
        let cap = compute_manpower_cap(grown);
        let expected = recover_manpower(0, cap);
        assert_eq!(
            app.world()
                .resource::<CountryRegistry>()
                .get(CountryId(1))
                .unwrap()
                .available_manpower,
            expected
        );
        assert!(expected > 0);
    }

    #[test]
    fn eliminated_country_does_not_recover_manpower_even_with_leftover_population_credit() {
        let mut app = build_test_app();
        // 消滅国家(1)が(本来あり得ないが)まだ州を所有しているという最悪ケースでも、
        // is_active()ゲートにより回復対象から除外されることを確認する。
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_000_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![eliminated_country(1, 2, 0), country(2, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        assert_eq!(
            app.world()
                .resource::<CountryRegistry>()
                .get(CountryId(1))
                .unwrap()
                .available_manpower,
            0,
            "an eliminated country must never recover manpower regardless of state ownership data"
        );
    }

    #[test]
    fn a_country_with_no_owned_states_does_not_recover_manpower() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 2, 1_000_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 500), country(2, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        assert_eq!(
            app.world()
                .resource::<CountryRegistry>()
                .get(CountryId(1))
                .unwrap()
                .available_manpower,
            500,
            "a country that owns no states has a zero manpower cap and must not gain manpower"
        );
    }

    #[test]
    fn manpower_recovery_follows_ownership_transfer_reflected_before_this_months_tick() {
        // 州所有権移転が「この月次計算より前」に反映されていれば、新所有者側で
        // 人口が集計され回復対象になる(WarResolution等、Economyより前のSetで
        // 所有権が変わった場合の挙動を固定する)。
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 2, 1_200_000)])); // 既にCountryId(2)へ移転済み
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0), country(2, 0)],
        });

        send_month_changed(&mut app);
        app.update();

        let registry = app.world().resource::<CountryRegistry>();
        assert_eq!(
            registry.get(CountryId(1)).unwrap().available_manpower,
            0,
            "the former owner must not be credited with population it no longer owns"
        );
        assert!(
            registry.get(CountryId(2)).unwrap().available_manpower > 0,
            "the new owner must be credited with the transferred state's population"
        );
    }

    /// 同じ日にState所有権移転(実ゲームでは`WarResolution` Setで起こる、講和settlement等)と
    /// 月替わりが重なった場合の反映タイミングを、実際に設定された`DailySimulationSet`の
    /// 順序(`app::time::GameTimePlugin`が`configure_sets`する本番の順序、
    /// `Economy`は`WarResolution`より前)を通して固定する。
    #[test]
    fn ownership_transfer_in_war_resolution_is_reflected_starting_the_following_months_tick() {
        use crate::app::game_state::GameState;
        use crate::app::time::{DailySimulationSet, GameTimePlugin};

        #[derive(Resource, Default)]
        struct TransferDone(bool);

        fn simulate_a_war_resolution_ownership_transfer(
            mut state_registry: ResMut<StateRegistry>,
            mut done: ResMut<TransferDone>,
        ) {
            if done.0 {
                return;
            }
            if let Some(s) = state_registry.get_mut(crate::common::StateId(0)) {
                s.owner_country_id = CountryId(2);
            }
            done.0 = true;
        }

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(StatesPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_state::<GameState>();
        app.insert_state(GameState::Playing);
        app.add_plugins(GameTimePlugin);
        app.insert_resource(TransferDone::default());
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_200_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0), country(2, 0)],
        });
        app.add_systems(
            Update,
            handle_monthly_population_growth
                .in_set(DailySimulationSet::Economy)
                .run_if(in_state(GameState::Playing)),
        );
        app.add_systems(
            Update,
            simulate_a_war_resolution_ownership_transfer
                .in_set(DailySimulationSet::WarResolution)
                .run_if(in_state(GameState::Playing)),
        );

        // フレーム1: 月替わりと所有権移転(WarResolution、Economyより後)が同じフレームで
        // 起こる。EconomyがWarResolutionより先に実行されるため、この月はまだ旧所有国
        // (CountryId(1))が人口を集計され、CountryId(2)はまだ何も得ない。
        send_month_changed(&mut app);
        app.update();

        {
            let registry = app.world().resource::<CountryRegistry>();
            assert!(
                registry.get(CountryId(1)).unwrap().available_manpower > 0,
                "the former owner must still be credited for the month the transfer itself \
                 happens on, since Economy runs before WarResolution"
            );
            assert_eq!(
                registry.get(CountryId(2)).unwrap().available_manpower,
                0,
                "the new owner must not be credited yet within the same frame the transfer \
                 happened in"
            );
        }

        // フレーム2: 次の月替わり。所有権は既に移転済みなので、今度はCountryId(2)が
        // 集計対象になる。
        send_month_changed(&mut app);
        app.update();

        let registry = app.world().resource::<CountryRegistry>();
        assert!(
            registry.get(CountryId(2)).unwrap().available_manpower > 0,
            "the new owner must be credited starting from the next month's tick"
        );
    }

    // ─── 長期挙動(150年 = 1800か月) ───────────────────────────────────────────

    #[test]
    fn eighteen_hundred_months_run_without_panicking_or_overflowing() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![
            state(0, 1, 1_200_000),
            state(1, 1, 25_000),
            state(2, 2, 2_100_000),
        ]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0), country(2, 0)],
        });

        for month in 0..1_800u32 {
            let mut writer: bevy::ecs::system::SystemState<MessageWriter<MonthChangedMessage>> =
                bevy::ecs::system::SystemState::new(app.world_mut());
            writer
                .get_mut(app.world_mut())
                .expect("writer")
                .write(MonthChangedMessage {
                    year: 1800 + (month / 12) as i32,
                    month: (month % 12) as u8 + 1,
                });
            writer.apply(app.world_mut());
            app.update();
        }

        let final_pop_0 = app
            .world()
            .resource::<StateRegistry>()
            .get(crate::common::StateId(0))
            .unwrap()
            .population;
        // 150年(1800か月)複利成長: 概ね開始時の2〜3倍(理論値 e^0.9 ≒ 2.4596倍)。
        assert!(
            (1_200_000 * 2..=1_200_000 * 3).contains(&final_pop_0),
            "150 years of growth must land within roughly 2x-3x the starting population, got {final_pop_0}"
        );

        let country1_manpower = app
            .world()
            .resource::<CountryRegistry>()
            .get(CountryId(1))
            .unwrap()
            .available_manpower;
        let cap = compute_manpower_cap(
            app.world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population
                + app
                    .world()
                    .resource::<StateRegistry>()
                    .get(crate::common::StateId(1))
                    .unwrap()
                    .population,
        );
        assert_eq!(
            country1_manpower, cap,
            "after 150 years (>> the 120-month recovery period) manpower must have fully \
             caught up to its population-derived cap"
        );
    }

    #[test]
    fn identical_starting_state_and_month_count_produce_identical_results() {
        fn run(months: u32) -> (u64, u64) {
            let mut app = build_test_app();
            app.insert_resource(StateRegistry::build(vec![state(0, 1, 837_421)]));
            app.insert_resource(CountryRegistry {
                countries: vec![country(1, 0)],
            });
            for _ in 0..months {
                send_month_changed(&mut app);
                app.update();
            }
            let pop = app
                .world()
                .resource::<StateRegistry>()
                .get(crate::common::StateId(0))
                .unwrap()
                .population;
            let manpower = app
                .world()
                .resource::<CountryRegistry>()
                .get(CountryId(1))
                .unwrap()
                .available_manpower;
            (pop, manpower)
        }

        assert_eq!(
            run(240),
            run(240),
            "identical starting state and identical month count must always produce identical \
             results (determinism)"
        );
    }

    #[test]
    fn empty_manpower_reaches_the_cap_within_the_configured_recovery_period() {
        let mut app = build_test_app();
        app.insert_resource(StateRegistry::build(vec![state(0, 1, 1_200_000)]));
        app.insert_resource(CountryRegistry {
            countries: vec![country(1, 0)],
        });

        for _ in 0..MANPOWER_RECOVERY_PERIOD_MONTHS as u32 {
            send_month_changed(&mut app);
            app.update();
        }

        let registry = app.world().resource::<CountryRegistry>();
        let population = app
            .world()
            .resource::<StateRegistry>()
            .get(crate::common::StateId(0))
            .unwrap()
            .population;
        let cap = compute_manpower_cap(population);
        let manpower = registry.get(CountryId(1)).unwrap().available_manpower;
        // population(したがってcap)自体も毎月わずかに成長し続ける移動目標のため、
        // 「概ね10年で基準上限まで回復する」という設計上の目安どおり、120か月時点で
        // 上限のかなりの割合(95%以上)まで追いついていることを確認する
        // (`eighteen_hundred_months_run_without_panicking_or_overflowing`で、
        // 十分な月数が経てば完全に追いつくこと自体は別途確認済み)。
        assert!(
            manpower * 100 >= cap * 95,
            "manpower must reach at least 95% of the (slightly moving) population cap within \
             the configured recovery period, got {manpower} of cap {cap}"
        );
    }
}
