//! P20-008: 1000州以上規模での本番日次シミュレーション性能プロファイリング用モジュール。
//!
//! `strategy_game/assets/data/states.ron` / `countries.ron` は一切変更・複製・巨大化せず、
//! ここでは決定論的に合成した大規模ワールド（State/Country/Army/War/Frontline）を
//! 本番のプラグイン構成・`DailySimulationSet` 実行順序へ直接注入する。
//! `src/bin/profile_1000_states.rs`（計測用バイナリ）と
//! `tests/profile_workload_correctness_test.rs`（正しさの回帰テスト）の両方から
//! 同一のワールド生成ロジックを再利用することで、計測経路と検証経路の乖離を防ぐ。

use crate::app::AppPlugin;
use crate::app::game_state::GameState;
use crate::app::time::{GameDate, GamePaused};
use crate::building::BuildingPlugin;
use crate::building::data::BuildingType;
use crate::common::{ArmyId, CountryId, DivisionId, StateId, WarId};
use crate::country::{
    CountryData, CountryPlugin, CountryRegistry, EconomicSystem, GovernmentType, PlayerCountry,
};
use crate::diplomacy::DiplomacyPlugin;
use crate::diplomacy::crisis::{WarGoal, WarGoalType};
use crate::diplomacy::data::{DiplomacyRegistry, DiplomaticPairKey, DiplomaticRelation};
use crate::economy::EconomyPlugin;
use crate::economy::resources::{CountryStockpile, ResourceType, StateResourceDeposit};
use crate::military::MilitaryPlugin;
use crate::military::battle::BattleRegistry;
use crate::military::data::{ArmyStatus, ArmyUnit, DivisionDefinition, MilitaryRegistry};
use crate::politics::PoliticsPlugin;
use crate::research::ResearchPlugin;
use crate::state::StatePlugin;
use crate::state::data::{StateData, StateRegistry};
use crate::war::WarPlugin;
use crate::war::data::{War, WarRegistry, WarStatus};
use crate::war::frontline::FrontlineRegistry;
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::app::time::DailySimulationSet;

/// 1国家あたりの平均State数（比率の根拠は `docs`/報告書参照）
pub const COUNTRY_STATE_RATIO: usize = 20;
/// 最小国家数（小規模ワールドでも国家間相互作用が発生するように下限を設ける）
pub const MIN_COUNTRIES: usize = 8;

/// プロファイリングシナリオ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// 通常シナリオ: 戦争なし、経済・研究・外交・国家AIの平時処理のみ
    Normal,
    /// 高負荷シナリオ: 複数戦争・軍事AI・前線処理を実際に稼働させる
    HighLoad,
}

impl Scenario {
    pub fn label(self) -> &'static str {
        match self {
            Scenario::Normal => "normal",
            Scenario::HighLoad => "high_load",
        }
    }
}

/// ワールド生成設定
#[derive(Debug, Clone, Copy)]
pub struct WorkloadConfig {
    pub state_count: usize,
    pub scenario: Scenario,
    /// 決定論的PRNGのSeed（固定すれば常に同じワールドが生成される）
    pub seed: u64,
}

/// 生成時に確定するワールド規模の統計
#[derive(Debug, Clone, Default)]
pub struct WorkloadCounts {
    pub state_count: usize,
    pub country_count: usize,
    pub initial_army_count: usize,
    pub initial_war_count: usize,
}

/// 実行時（Tick後）に観測するワールド統計
#[derive(Debug, Clone, Default)]
pub struct RuntimeCounts {
    pub army_count: usize,
    pub active_war_count: usize,
    pub frontline_count: usize,
    pub battle_count: usize,
    /// BevyのECS World上に実在するEntity数。
    /// 本ゲームのState/Country/Army/War/FrontlineデータはECS Entityではなく
    /// Registry Resource内のVec/HashMapとして保持されるため、この値は
    /// ワールド規模に応じて増加しない(常に少数)。規模の指標としては
    /// State/Country/Army/War/Frontline件数を参照すること。
    pub ecs_entity_count: usize,
}

/// 追加の依存クレートを増やさないための最小決定論的PRNG (SplitMix64)。
/// 同一Seedから常に同一列を生成するため、"乱数Seedを固定する" 要件を満たす。
pub struct DeterministicRng(u64);

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// [lo, hi) の範囲の値を返す
    pub fn next_range(&mut self, lo: u64, hi: u64) -> u64 {
        let span = hi.saturating_sub(lo).max(1);
        lo + self.next_u64() % span
    }
}

/// State数から国家数を決定する（比率は `COUNTRY_STATE_RATIO` を参照）
pub fn country_count_for(state_count: usize) -> usize {
    (state_count / COUNTRY_STATE_RATIO).max(MIN_COUNTRIES)
}

fn grid_side(state_count: usize) -> usize {
    ((state_count as f64).sqrt().ceil() as usize).max(1)
}

/// 本番と同一のプラグイン構成・`DailySimulationSet` 順序でAppを構築し、
/// 合成ワールドを注入して `GameState::Playing` へ遷移させる。
/// (`tests/daily_system_integration_test.rs::setup_test_app` と同一のプラグイン集合)
pub fn build_app(config: &WorkloadConfig) -> (App, WorkloadCounts) {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_once()))
        .add_plugins(bevy::state::app::StatesPlugin)
        .init_resource::<ButtonInput<KeyCode>>()
        .add_plugins(AppPlugin)
        .add_plugins(CountryPlugin)
        .add_plugins(StatePlugin)
        .add_plugins(BuildingPlugin)
        .add_plugins(EconomyPlugin)
        .add_plugins(ResearchPlugin)
        .add_plugins(PoliticsPlugin)
        .add_plugins(DiplomacyPlugin)
        .add_plugins(WarPlugin)
        .add_plugins(MilitaryPlugin);

    // Startup実行: 本番の小規模RON (buildings/technologies/world_stages/divisions等) を
    // 読み込む。states.ron/countries.ronの小規模実データもここで読み込まれるが、
    // 直後に合成ワールドで上書きするため無害。
    app.update();

    let counts = inject_synthetic_world(&mut app, config);

    app.insert_state(GameState::Playing);
    app.update();

    (app, counts)
}

#[allow(clippy::too_many_lines)]
fn inject_synthetic_world(app: &mut App, config: &WorkloadConfig) -> WorkloadCounts {
    let state_count = config.state_count.max(1);
    let country_count = country_count_for(state_count);
    let side = grid_side(state_count);
    let mut rng = DeterministicRng::new(config.seed);

    let country_cols = ((country_count as f64).sqrt().ceil() as usize).max(1);
    let country_rows = country_count.div_ceil(country_cols).max(1);

    // ── 1. State生成（グリッド配置、4近傍、国家ブロック分割） ──────────────────
    let mut states: Vec<StateData> = Vec::with_capacity(state_count);
    let mut country_of_state: Vec<CountryId> = Vec::with_capacity(state_count);

    for idx in 0..state_count {
        let row = idx / side;
        let col = idx % side;
        let crow = (row * country_rows) / side;
        let ccol = (col * country_cols) / side;
        let cidx = (crow * country_cols + ccol).min(country_count - 1);
        country_of_state.push(CountryId(cidx));

        let mut neighbors = Vec::new();
        if col > 0 {
            neighbors.push(StateId(idx - 1));
        }
        if col + 1 < side && idx + 1 < state_count {
            neighbors.push(StateId(idx + 1));
        }
        if row > 0 {
            neighbors.push(StateId(idx - side));
        }
        if idx + side < state_count {
            neighbors.push(StateId(idx + side));
        }

        let population = 20_000 + rng.next_range(0, 200_000);

        let mut buildings = HashMap::new();
        buildings.insert(BuildingType::Farm, 1 + (idx % 3) as u32);
        buildings.insert(BuildingType::Mine, 1 + (idx % 2) as u32);

        let res_variants = ResourceType::ALL;
        let deposit = StateResourceDeposit {
            resource_type: res_variants[idx % res_variants.len()],
            base_output: 5.0 + (idx % 10) as f64,
            discovered: true,
            development_level: 1,
        };

        states.push(StateData {
            id: StateId(idx),
            name: format!("Synthetic State {idx}"),
            owner_country_id: country_of_state[idx],
            population,
            neighbors,
            world_position: [col as f32 * 100.0, row as f32 * 100.0],
            size: [100.0, 100.0],
            buildings,
            resource_deposits: vec![deposit],
            ..Default::default()
        });
    }

    // ── 2. 首都決定（各国最初のState） ───────────────────────────────────────
    let mut capital_of_country: Vec<Option<StateId>> = vec![None; country_count];
    for (idx, cid) in country_of_state.iter().enumerate() {
        let cid = cid.0;
        if capital_of_country[cid].is_none() {
            capital_of_country[cid] = Some(StateId(idx));
        }
    }

    // ── 3. Country生成 ──────────────────────────────────────────────────────
    let gov_types = [
        GovernmentType::Monarchy,
        GovernmentType::Republic,
        GovernmentType::Dictatorship,
        GovernmentType::Theocracy,
    ];
    let eco_types = [
        EconomicSystem::FreeMarket,
        EconomicSystem::Mercantilism,
        EconomicSystem::StateCapitalism,
        EconomicSystem::Socialism,
    ];

    let mut countries: Vec<CountryData> = Vec::with_capacity(country_count);
    for cidx in 0..country_count {
        let capital = capital_of_country[cidx].unwrap_or(StateId(0));
        let mut stockpile = CountryStockpile::new();
        for res in ResourceType::ALL {
            stockpile.set(res, 300.0 + rng.next_range(0, 500) as f64);
        }

        countries.push(CountryData {
            id: CountryId(cidx),
            name: format!("Synthetic Nation {cidx}"),
            map_color: [0.5, 0.5, 0.5],
            capital_state_id: capital,
            treasury: 2000.0 + rng.next_range(0, 3000) as f64,
            government_type: gov_types[cidx % gov_types.len()],
            economic_system: eco_types[cidx % eco_types.len()],
            stockpile,
            ..Default::default()
        });
    }

    // ── 4. 高負荷シナリオ用: 隣接国ブロックペアで戦争を準備 ──────────────────────
    let mut wars: Vec<War> = Vec::new();
    // (country, state, is_attacker) の初期陸軍配置リスト
    let mut army_placements: Vec<(CountryId, StateId)> = Vec::new();

    // 平時ガリソン: 全国家に首都1個師団
    for (cidx, cap) in capital_of_country.iter().enumerate() {
        if let Some(cap) = cap {
            army_placements.push((CountryId(cidx), *cap));
        }
    }

    if config.scenario == Scenario::HighLoad {
        // 国家ブロックの4近傍ペアを列挙
        let mut block_pairs: Vec<(usize, usize)> = Vec::new();
        for r in 0..country_rows {
            for c in 0..country_cols {
                let a = r * country_cols + c;
                if a >= country_count {
                    continue;
                }
                if c + 1 < country_cols {
                    let b = r * country_cols + (c + 1);
                    if b < country_count {
                        block_pairs.push((a, b));
                    }
                }
                if r + 1 < country_rows {
                    let b = (r + 1) * country_cols + c;
                    if b < country_count {
                        block_pairs.push((a, b));
                    }
                }
            }
        }

        let war_pair_count = (country_count / 10).max(1).min(block_pairs.len());
        let mut used_countries: HashSet<usize> = HashSet::new();

        for &(a, b) in block_pairs.iter() {
            if wars.len() >= war_pair_count {
                break;
            }
            if used_countries.contains(&a) || used_countries.contains(&b) {
                continue;
            }

            // a(攻撃側)が所有するStateでbが所有するStateと隣接する国境ペアを探索
            let mut border_pair: Option<(StateId, StateId)> = None;
            for idx in 0..state_count {
                if country_of_state[idx].0 != a {
                    continue;
                }
                for &nb in &states[idx].neighbors {
                    if country_of_state[nb.0].0 == b {
                        border_pair = Some((StateId(idx), nb));
                        break;
                    }
                }
                if border_pair.is_some() {
                    break;
                }
            }

            let Some((atk_state, def_state)) = border_pair else {
                continue;
            };

            used_countries.insert(a);
            used_countries.insert(b);

            let mut attackers = HashSet::new();
            attackers.insert(CountryId(a));
            let mut defenders = HashSet::new();
            defenders.insert(CountryId(b));

            wars.push(War {
                id: WarId(0), // add_war 時に採番される
                name: format!("Synthetic War {a} vs {b}"),
                attackers,
                defenders,
                war_goals: vec![WarGoal {
                    attacker: CountryId(a),
                    defender: CountryId(b),
                    goal_type: WarGoalType::ConquerState,
                    target_states: vec![def_state],
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
                occupied_states: HashSet::new(),
                status: WarStatus::Active,
                winner: None,
                end_reason: None,
                applied_terms: Vec::new(),
                won_attacker_battles: 0,
                won_defender_battles: 0,
                processed_battle_ids: HashSet::new(),
            });

            // 交戦国双方に追加の陸軍を国境州・首都へ配置
            for _ in 0..4 {
                army_placements.push((CountryId(a), atk_state));
                army_placements.push((CountryId(b), def_state));
            }
        }
    }

    let initial_war_count = wars.len();
    let initial_army_count = army_placements.len();

    // ── 5. Diplomacy: 全国家ペアの関係を生成 ────────────────────────────────
    let mut diplomacy = DiplomacyRegistry::default();
    for a in 0..country_count {
        for b in (a + 1)..country_count {
            if let Some(key) = DiplomaticPairKey::new(CountryId(a), CountryId(b)) {
                diplomacy
                    .relations
                    .insert(key, DiplomaticRelation::default());
            }
        }
    }

    // ── 6. ECSリソースへ反映 ─────────────────────────────────────────────────
    app.insert_resource(StateRegistry::build(states));
    app.insert_resource(CountryRegistry { countries });
    app.insert_resource(PlayerCountry(Some(CountryId(0))));
    app.insert_resource(diplomacy);

    {
        let mut war_registry = app.world_mut().resource_mut::<WarRegistry>();
        for war in wars {
            war_registry.add_war(war);
        }
    }

    {
        // 実データ(divisions.ron)から読み込まれた定義を再利用する
        let def: DivisionDefinition = {
            let military = app.world().resource::<MilitaryRegistry>();
            military
                .definitions
                .get(&DivisionId(0))
                .cloned()
                .expect("divisions.ron に DivisionId(0) が定義されていること")
        };
        let def_id = DivisionId(0);

        let mut military = app.world_mut().resource_mut::<MilitaryRegistry>();
        military.armies.clear();
        for (owner, state) in army_placements {
            let army = ArmyUnit {
                id: ArmyId(0), // add_army 時に採番される
                owner,
                division_type: def.division_type,
                size: def.size,
                current_state: state,
                destination: None,
                current_path: Vec::new(),
                target_state: None,
                manpower: def.required_manpower,
                max_manpower: def.required_manpower,
                equipment: def.required_equipment,
                max_equipment: def.required_equipment,
                organization: def.organization,
                max_organization: def.organization,
                morale: def.morale,
                max_morale: def.morale,
                experience: 0.0,
                supply_ratio: 1.0,
                movement_progress: 0.0,
                status: ArmyStatus::Idle,
                def_id,
                attack_power: def.attack as i32,
                defense_power: def.defense as i32,
                combat_id: None,
            };
            military.add_army(army);
        }
    }

    WorkloadCounts {
        state_count,
        country_count,
        initial_army_count,
        initial_war_count,
    }
}

/// 1日分の日次シミュレーションを進める。
/// `tests/daily_system_integration_test.rs::advance_day_by_system` と同一の手法
/// (`GameDate` のaccumulatorへ直接1.0を加算し、実時間待機を伴わず決定論的に1日進める)。
pub fn advance_one_day(app: &mut App) {
    app.world_mut().resource_mut::<GamePaused>().0 = false;
    app.world_mut()
        .resource_mut::<GameDate>()
        .add_accumulator(1.0);
    app.update();
}

/// 実行時点のワールド統計を取得する
pub fn snapshot_runtime_counts(app: &App) -> RuntimeCounts {
    let world = app.world();
    let military = world.resource::<MilitaryRegistry>();
    let war_registry = world.resource::<WarRegistry>();
    let frontline_registry = world.resource::<FrontlineRegistry>();
    let battle_registry = world.resource::<BattleRegistry>();

    RuntimeCounts {
        army_count: military.armies.len(),
        active_war_count: war_registry
            .wars
            .values()
            .filter(|w| w.status == WarStatus::Active)
            .count(),
        frontline_count: frontline_registry.frontlines.len(),
        battle_count: battle_registry.battles.len(),
        ecs_entity_count: world.entities().len() as usize,
    }
}

/// 全国家の国庫合計（正しさ検証・決定論比較用）
pub fn total_treasury(app: &App) -> f64 {
    app.world()
        .resource::<CountryRegistry>()
        .countries
        .iter()
        .map(|c| c.treasury)
        .sum()
}

/// 全State人口合計（正しさ検証・決定論比較用）
pub fn total_population(app: &App) -> u64 {
    app.world()
        .resource::<StateRegistry>()
        .states
        .iter()
        .map(|s| s.population)
        .sum()
}

// ─────────────────────────────────────────────────────────────────────────
// SystemSet別タイミング計測
//
// 本番コード（`app::time::GameTimePlugin` 他）は一切変更せず、
// 各 `DailySimulationSet` の境界へ `.before(Set)` / `.after(Set)` 制約付きの
// 軽量マーカーSystemを追加registerすることで、既存の `configure_sets(...).chain()`
// 順序を利用してSet単位の所要時間を計測する。
// ─────────────────────────────────────────────────────────────────────────

/// `DailySimulationSet` の並び順そのまま（境界計測のラベルとして使用）
pub const DAILY_SET_NAMES: [&str; 11] = [
    "TimeUpdate",
    "Economy",
    "Research",
    "Diplomacy",
    "CountryAi",
    "WarPreparation",
    "MilitaryAi",
    "FrontlineOrders",
    "MilitaryAction",
    "WarResolution",
    "UiUpdate",
];

/// 日次SystemSet境界のタイムスタンプ（毎フレーム12件: Set境界0..=11）
#[derive(Resource, Default)]
pub struct SetTimings {
    pub checkpoints: Vec<Instant>,
}

fn mark_reset_and_start(mut timings: ResMut<SetTimings>) {
    timings.checkpoints.clear();
    timings.checkpoints.push(Instant::now());
}

fn mark(mut timings: ResMut<SetTimings>) {
    timings.checkpoints.push(Instant::now());
}

/// SystemSet境界計測用マーカーSystemを登録する。
/// 本番の `DailySimulationSet` チェーン順序（`app::time::GameTimePlugin::build` 参照）は
/// 変更せず、その前後に観測点を挿入するのみ。
pub fn install_set_timings(app: &mut App) {
    use DailySimulationSet as S;
    app.init_resource::<SetTimings>();
    app.add_systems(
        Update,
        (
            mark_reset_and_start.before(S::TimeUpdate),
            mark.after(S::TimeUpdate).before(S::Economy),
            mark.after(S::Economy).before(S::Research),
            mark.after(S::Research).before(S::Diplomacy),
            mark.after(S::Diplomacy).before(S::CountryAi),
            mark.after(S::CountryAi).before(S::WarPreparation),
            mark.after(S::WarPreparation).before(S::MilitaryAi),
            mark.after(S::MilitaryAi).before(S::FrontlineOrders),
            mark.after(S::FrontlineOrders).before(S::MilitaryAction),
            mark.after(S::MilitaryAction).before(S::WarResolution),
            mark.after(S::WarResolution).before(S::UiUpdate),
            mark.after(S::UiUpdate),
        ),
    );
}

/// 直近フレームのSet別所要時間を取得する（12個のチェックポイントから11区間を算出）。
/// チェックポイント数が不正な場合（Playing状態でなかった等）は `None`。
pub fn read_set_durations(app: &App) -> Option<[std::time::Duration; 11]> {
    let timings = app.world().resource::<SetTimings>();
    if timings.checkpoints.len() != 12 {
        return None;
    }
    let mut durations = [std::time::Duration::ZERO; 11];
    for (d, w) in durations.iter_mut().zip(timings.checkpoints.windows(2)) {
        *d = w[1] - w[0];
    }
    Some(durations)
}

/// NaN・無限値・無効ID参照が発生していないことを検証する。
/// 問題があれば人間可読なエラーメッセージを返す。
pub fn validate_world_sanity(app: &App) -> Result<(), String> {
    let world = app.world();
    let state_registry = world.resource::<StateRegistry>();
    let country_registry = world.resource::<CountryRegistry>();
    let military = world.resource::<MilitaryRegistry>();

    for c in &country_registry.countries {
        if !c.treasury.is_finite() {
            return Err(format!(
                "Country {:?} treasury is not finite: {}",
                c.id, c.treasury
            ));
        }
        for (res, amount) in &c.stockpile.amounts {
            if !amount.is_finite() {
                return Err(format!(
                    "Country {:?} stockpile[{:?}] is not finite: {}",
                    c.id, res, amount
                ));
            }
        }
        if state_registry.get(c.capital_state_id).is_none() {
            return Err(format!(
                "Country {:?} capital_state_id {:?} does not reference an existing state",
                c.id, c.capital_state_id
            ));
        }
    }

    for s in &state_registry.states {
        if !s.living_standard.is_finite() || !s.unrest.is_finite() || !s.logistics_ratio.is_finite()
        {
            return Err(format!("State {:?} has non-finite derived value", s.id));
        }
        if country_registry.get(s.owner_country_id).is_none() {
            return Err(format!(
                "State {:?} owner_country_id {:?} does not reference an existing country",
                s.id, s.owner_country_id
            ));
        }
        for &n in &s.neighbors {
            if state_registry.get(n).is_none() {
                return Err(format!(
                    "State {:?} neighbor {:?} does not reference an existing state",
                    s.id, n
                ));
            }
        }
    }

    for army in military.armies.values() {
        if country_registry.get(army.owner).is_none() {
            return Err(format!(
                "Army {:?} owner {:?} does not reference an existing country",
                army.id, army.owner
            ));
        }
        if state_registry.get(army.current_state).is_none() {
            return Err(format!(
                "Army {:?} current_state {:?} does not reference an existing state",
                army.id, army.current_state
            ));
        }
        if !army.organization.is_finite() || !army.morale.is_finite() {
            return Err(format!("Army {:?} has non-finite derived value", army.id));
        }
    }

    Ok(())
}
