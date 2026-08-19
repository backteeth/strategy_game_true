//! P21-014: 国家総合力評価・国家ランク基盤。
//!
//! 各国家の軍事力・経済力・人口を実データから算出し、世界内で0.0〜100.0へ正規化した
//! 上で、決定論的な世界順位と国家ランク(大国/地域大国/小国)を付与する。
//!
//! Country/State/Military/Buildingの各Registryから毎回再構築する派生データであり、
//! Saveへは保存しない(New Game直後・Load直後・月次進行のタイミングで再構築される。
//! 詳細は`crate::country::rebuild_country_power_registry_from_world`と
//! `crate::country::CountryPlugin`を参照)。
//!
//! Crisis支持資格の制限・AIによる国家ランク利用はP21-015以降の対象であり、本モジュールは
//! 評価・順位・ランク・参照APIのみを提供する。

use crate::app::time::{GameDate, MonthChangedMessage};
use crate::building::data::{BuildingRegistry, BuildingType};
use crate::common::CountryId;
use crate::country::CountryRegistry;
use crate::country::country_ai::compute_total_power_by_country;
use crate::economy::resources::ResourceType;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use bevy::prelude::*;
use std::collections::HashMap;

/// 国家ランク(大国/地域大国/小国)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerTier {
    GreatPower,
    RegionalPower,
    MinorPower,
}

impl PowerTier {
    /// 表示用の翻訳キー(P20-009の`display_name`規約に合わせる)。
    pub fn display_name(self) -> &'static str {
        match self {
            PowerTier::GreatPower => "power_tier.great_power",
            PowerTier::RegionalPower => "power_tier.regional_power",
            PowerTier::MinorPower => "power_tier.minor_power",
        }
    }
}

/// 1国家分の国家総合力評価結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountryPowerAssessment {
    pub country_id: CountryId,
    /// 正規化前の生値。
    pub military_raw: f64,
    pub economic_raw: f64,
    pub population_raw: f64,
    /// 世界最大値を100とした0.0〜100.0の正規化値。
    pub military_normalized: f32,
    pub economic_normalized: f32,
    pub population_normalized: f32,
    /// 50/30/20の重み付き合計 (0.0〜100.0)。
    pub total_score: f32,
    /// 1始まりの世界順位。
    pub world_rank: usize,
    pub power_tier: PowerTier,
}

/// 全国家の国家総合力評価結果を保持するリソース。
///
/// Country/State/Military/Buildingから`evaluate_country_power`で毎回再構築する派生
/// データであり、この型自体を外部から直接書き換える公開APIは提供しない
/// (`CountryPlugin`経由の再評価のみが正規の更新経路)。
#[derive(Resource, Debug, Default, Clone)]
pub struct CountryPowerRegistry {
    assessments: HashMap<CountryId, CountryPowerAssessment>,
    /// 世界順位順(1位から)に並んだCountryId一覧。
    ordered_country_ids: Vec<CountryId>,
    last_evaluated_date: Option<String>,
}

impl CountryPowerRegistry {
    /// O(1)で1国家分の評価を取得する。
    pub fn get(&self, id: CountryId) -> Option<&CountryPowerAssessment> {
        self.assessments.get(&id)
    }

    /// 世界順位順(1位から)に並んだCountryId一覧。
    pub fn ordered_country_ids(&self) -> &[CountryId] {
        &self.ordered_country_ids
    }

    /// 評価対象国家数(=世界順位の最大値)。
    pub fn country_count(&self) -> usize {
        self.ordered_country_ids.len()
    }

    /// 直近の評価日(表示用文字列)。一度も評価されていない場合は`None`
    /// (`MainMenu`/`CountrySelection`中などUI側が「評価中」を表示すべき状態)。
    pub fn last_evaluated_date(&self) -> Option<&str> {
        self.last_evaluated_date.as_deref()
    }
}

/// 有限値かつ非負であることを保証する(NaN・無限大・負値を総合力へ伝播させない)。
fn sanitize_raw(x: f64) -> f64 {
    if x.is_finite() { x.max(0.0) } else { 0.0 }
}

/// `raw`を世界最大値`world_max`に対して0.0〜100.0へ正規化する。
/// `world_max <= 0.0`(全国家0)の場合は0.0を返す(0除算を避ける)。
fn normalize(raw: f64, world_max: f64) -> f32 {
    if world_max > 0.0 {
        ((raw / world_max) * 100.0).clamp(0.0, 100.0) as f32
    } else {
        0.0
    }
}

/// 国家数`n`から(大国数, 地域大国数)を決定する。残りは自動的に小国となる。
/// `n == 0`の場合は`(0, 0)`(空Registry、呼び出し元で早期returnする)。
fn compute_tier_counts(n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let great = (n * 20).div_ceil(100).clamp(1, 8).min(n);
    let remaining = n - great;
    let regional = (n * 30).div_ceil(100).min(remaining);
    (great, regional)
}

/// 州レジストリを1パスだけ走査し、所有国(owner、controllerではない)ごとの
/// 人口合計・経済力生値合計を同時に集計する(`countries × states`の総当たりを避ける)。
///
/// 経済力は「安定した生産能力」(建物レベル×建物定義の基礎産出量。現在の稼働率・
/// 国庫残高・備蓄量には一切依存しない)。優先順位:
/// - 建物定義`output_resources`が非空 (Farm/LoggingCamp/Factory/MilitaryFactory/
///   CrystalMine/CrystalRefinery): 出力資源ごとの基礎産出量合計 × レベル
/// - `Mine`(建物定義自体は`output_resources`を持たず、実際の産出は州の鉱床データに
///   依存する): `economy::production::process_country_production`と同じ規約で、
///   discovered済みかつMagicCrystal以外の鉱床の`base_output`合計 × レベル
/// - 上記いずれにも該当しない建物(Railway/University/MagicAcademy: 出力は
///   物流ボーナス・研究力/魔法力であり、経済生産物ではない)は0(除外)
fn compute_state_aggregates(
    state_registry: &StateRegistry,
    building_registry: &BuildingRegistry,
) -> (HashMap<CountryId, f64>, HashMap<CountryId, f64>) {
    let mut economic_by_country: HashMap<CountryId, f64> = HashMap::new();
    let mut population_by_country: HashMap<CountryId, f64> = HashMap::new();

    for state in &state_registry.states {
        let owner = state.owner_country_id;
        *population_by_country.entry(owner).or_insert(0.0) += state.population as f64;

        let mut state_economic = 0.0f64;
        for (&b_type, &level) in &state.buildings {
            if level == 0 {
                continue;
            }
            if b_type == BuildingType::Mine {
                for deposit in &state.resource_deposits {
                    if deposit.discovered && deposit.resource_type != ResourceType::MagicCrystal {
                        state_economic += deposit.base_output * level as f64;
                    }
                }
            } else if let Some(def) = building_registry.get(b_type)
                && !def.output_resources.is_empty()
            {
                let per_level: f64 = def.output_resources.values().sum();
                state_economic += per_level * level as f64;
            }
        }
        *economic_by_country.entry(owner).or_insert(0.0) += state_economic;
    }

    (economic_by_country, population_by_country)
}

/// 全国家の国家総合力を評価し、`CountryPowerRegistry`を構築する。
///
/// `country_registry.countries`に存在する国家だけを評価対象とする(Stateのowner
/// 参照が壊れていて実在しない国家を指していても、新規の評価エントリは作らない —
/// 集計自体は`HashMap`へ行うが、最終的な組み立ては`country_registry.countries`だけを
/// 走査するため、存在しないCountryIdは自然に無視される)。
///
/// 計算量: `O(countries + states + military entities + countries log countries)`。
/// 軍事力は`country_ai::compute_total_power_by_country`(既存のP20-008最適化済み一括
/// 集計API)をそのまま再利用し、`countries × military`の総当たりを避ける。経済力・
/// 人口は`state_registry`を1パスだけ走査する。
pub fn evaluate_country_power(
    country_registry: &CountryRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    building_registry: &BuildingRegistry,
    evaluated_date: String,
) -> CountryPowerRegistry {
    if country_registry.countries.is_empty() {
        return CountryPowerRegistry {
            assessments: HashMap::new(),
            ordered_country_ids: Vec::new(),
            last_evaluated_date: Some(evaluated_date),
        };
    }

    let military_by_country = compute_total_power_by_country(military_registry, state_registry);
    let (economic_by_country, population_by_country) =
        compute_state_aggregates(state_registry, building_registry);

    struct RawEntry {
        id: CountryId,
        mil: f64,
        eco: f64,
        pop: f64,
    }

    let raw: Vec<RawEntry> = country_registry
        .countries
        .iter()
        .map(|c| {
            let mil = sanitize_raw(military_by_country.get(c.id.0).copied().unwrap_or(0) as f64);
            let eco = sanitize_raw(economic_by_country.get(&c.id).copied().unwrap_or(0.0));
            let pop = sanitize_raw(population_by_country.get(&c.id).copied().unwrap_or(0.0));
            RawEntry {
                id: c.id,
                mil,
                eco,
                pop,
            }
        })
        .collect();

    let world_max_mil = raw.iter().map(|r| r.mil).fold(0.0, f64::max);
    let world_max_eco = raw.iter().map(|r| r.eco).fold(0.0, f64::max);
    let world_max_pop = raw.iter().map(|r| r.pop).fold(0.0, f64::max);

    let mut assessments: Vec<CountryPowerAssessment> = raw
        .iter()
        .map(|r| {
            let military_normalized = normalize(r.mil, world_max_mil);
            let economic_normalized = normalize(r.eco, world_max_eco);
            let population_normalized = normalize(r.pop, world_max_pop);
            let total_score = ((military_normalized as f64) * 0.50
                + (economic_normalized as f64) * 0.30
                + (population_normalized as f64) * 0.20)
                .clamp(0.0, 100.0) as f32;
            CountryPowerAssessment {
                country_id: r.id,
                military_raw: r.mil,
                economic_raw: r.eco,
                population_raw: r.pop,
                military_normalized,
                economic_normalized,
                population_normalized,
                total_score,
                world_rank: 0,
                power_tier: PowerTier::MinorPower,
            }
        })
        .collect();

    // 決定論的な全順序: 総合力降順 → 軍事力正規化値降順 → 経済力正規化値降順 →
    // 人口正規化値降順 → CountryId昇順。`total_cmp`はNaNを含めても常に全順序を返すため
    // (この時点で既に`sanitize_raw`/`normalize`によりNaNは発生し得ないが、防御的に)
    // `partial_cmp().unwrap()`のようなpanicの可能性を排除する。
    assessments.sort_by(|a, b| {
        b.total_score
            .total_cmp(&a.total_score)
            .then_with(|| b.military_normalized.total_cmp(&a.military_normalized))
            .then_with(|| b.economic_normalized.total_cmp(&a.economic_normalized))
            .then_with(|| b.population_normalized.total_cmp(&a.population_normalized))
            .then_with(|| a.country_id.0.cmp(&b.country_id.0))
    });

    let n = assessments.len();
    let (great_n, regional_n) = compute_tier_counts(n);
    for (idx, a) in assessments.iter_mut().enumerate() {
        a.world_rank = idx + 1;
        a.power_tier = if idx < great_n {
            PowerTier::GreatPower
        } else if idx < great_n + regional_n {
            PowerTier::RegionalPower
        } else {
            PowerTier::MinorPower
        };
    }

    let ordered_country_ids: Vec<CountryId> = assessments.iter().map(|a| a.country_id).collect();
    let assessments: HashMap<CountryId, CountryPowerAssessment> =
        assessments.into_iter().map(|a| (a.country_id, a)).collect();

    CountryPowerRegistry {
        assessments,
        ordered_country_ids,
        last_evaluated_date: Some(evaluated_date),
    }
}

/// New Gameの`OnEnter(GameState::Playing)`チェーンの一部として、初回評価を構築する。
/// `app::loader::spawn_debug_divisions`の後に`.chain()`で実行する必要がある
/// (デバッグ初期師団配置後の軍事力を評価対象に含めるため)。Load-from-CountrySelection
/// (起動直後の「続きから」)もこの`OnEnter`を経由するが、`apply_validated_save`側の
/// `rebuild_country_power_registry_from_world`で既に再構築済みのため、ここでの
/// 再実行は冪等な二重評価に留まる(実害はない)。
pub fn rebuild_country_power_on_enter_playing(
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    building_registry: Res<BuildingRegistry>,
    date: Res<GameDate>,
    mut power_registry: ResMut<CountryPowerRegistry>,
) {
    *power_registry = evaluate_country_power(
        &country_registry,
        &state_registry,
        &military_registry,
        &building_registry,
        date.display(),
    );
}

/// 月次進行(`MonthChangedMessage`)ごとに再評価する。Pause中は`app::time::advance_game_date`が
/// メッセージ自体を発行しないため、この`MessageReader`は自然に空になり再評価も行われない。
/// `DailySimulationSet::UiUpdate`(最終Set)に配置し、同じフレーム内のEconomy/Diplomacy/
/// CountryAi/War各Setでの変更を必ず反映した状態で評価する。同一フレームに複数の
/// `MonthChangedMessage`が積まれていても(通常は起こらない)1回だけ再評価する。
pub fn rebuild_country_power_monthly(
    mut month_events: MessageReader<MonthChangedMessage>,
    country_registry: Res<CountryRegistry>,
    state_registry: Res<StateRegistry>,
    military_registry: Res<MilitaryRegistry>,
    building_registry: Res<BuildingRegistry>,
    date: Res<GameDate>,
    mut power_registry: ResMut<CountryPowerRegistry>,
) {
    if month_events.read().next().is_none() {
        return;
    }
    *power_registry = evaluate_country_power(
        &country_registry,
        &state_registry,
        &military_registry,
        &building_registry,
        date.display(),
    );
}

/// Save適用成功直後に呼ぶための`&mut World`版。`save::apply::apply_validated_save`から
/// 呼ばれる。ロード直後は次の月次進行を待たず、最初の通常Updateで正しい評価値を
/// 参照できる必要がある(ゲーム内`GameState::Playing`への状態遷移を伴わない、
/// 既にPlaying中からの「ロード」操作は`OnEnter(GameState::Playing)`を経由しないため、
/// この呼び出しが再構築の唯一の経路になる)。
pub fn rebuild_country_power_registry_from_world(world: &mut World) {
    let new_registry = {
        let country_registry = world.resource::<CountryRegistry>();
        let state_registry = world.resource::<StateRegistry>();
        let military_registry = world.resource::<MilitaryRegistry>();
        let building_registry = world.resource::<BuildingRegistry>();
        let date = world.resource::<GameDate>();
        evaluate_country_power(
            country_registry,
            state_registry,
            military_registry,
            building_registry,
            date.display(),
        )
    };
    world.insert_resource(new_registry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::building::data::BuildingDefinition;
    use crate::common::StateId;
    use crate::country::CountryData;
    use crate::economy::resources::StateResourceDeposit;
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    use crate::state::data::StateData;

    fn country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            ..CountryData::default()
        }
    }

    fn countries(ids: &[usize]) -> CountryRegistry {
        CountryRegistry {
            countries: ids.iter().map(|&id| country(id)).collect(),
        }
    }

    fn state(id: usize, owner: usize, population: u64) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            population,
            ..StateData::default()
        }
    }

    fn states(list: Vec<StateData>) -> StateRegistry {
        StateRegistry::build(list)
    }

    fn building_def(b_type: BuildingType, output: &[(ResourceType, f64)]) -> BuildingDefinition {
        BuildingDefinition {
            building_type: b_type,
            name: "test".to_string(),
            construction_cost: 0.0,
            required_progress: 0.0,
            required_workforce: 0.0,
            logistics_cost: 0.0,
            input_resources: HashMap::new(),
            output_resources: output.iter().copied().collect(),
            maintenance_cost: 0.0,
            max_level: 10,
            science_output: 0.0,
            magic_output: 0.0,
            railway_capacity_bonus: 0.0,
        }
    }

    fn building_registry() -> BuildingRegistry {
        let mut definitions = HashMap::new();
        definitions.insert(
            BuildingType::Farm,
            building_def(BuildingType::Farm, &[(ResourceType::Food, 100.0)]),
        );
        definitions.insert(
            BuildingType::LoggingCamp,
            building_def(BuildingType::LoggingCamp, &[(ResourceType::Wood, 80.0)]),
        );
        definitions.insert(BuildingType::Mine, building_def(BuildingType::Mine, &[]));
        definitions.insert(
            BuildingType::Factory,
            building_def(
                BuildingType::Factory,
                &[(ResourceType::IndustrialGoods, 30.0)],
            ),
        );
        definitions.insert(
            BuildingType::Railway,
            building_def(BuildingType::Railway, &[]),
        );
        definitions.insert(
            BuildingType::University,
            building_def(BuildingType::University, &[]),
        );
        BuildingRegistry { definitions }
    }

    fn division(id: usize, owner: usize, state_id: usize, manpower: u64) -> Division {
        Division {
            id: crate::common::DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(state_id),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower: manpower.max(1),
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
        }
    }

    fn military_with(divisions: Vec<Division>) -> MilitaryRegistry {
        let mut registry = MilitaryRegistry::default();
        for d in divisions {
            registry.divisions.insert(d.id, d);
        }
        registry
    }

    // ─── 軍事力 ─────────────────────────────────────────────────────────────

    /// 要求テスト1: 配備軍を持つ国家の軍事力が0より大きい。
    /// 要求テスト2: 軍を持たない国家は軍事力0。
    #[test]
    fn military_raw_is_positive_with_divisions_and_zero_without() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![state(1, 0, 1000), state(2, 1, 1000)]);
        let military_registry = military_with(vec![division(1, 0, 1, 10_000)]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert!(registry.get(CountryId(0)).unwrap().military_raw > 0.0);
        assert_eq!(registry.get(CountryId(1)).unwrap().military_raw, 0.0);
    }

    /// 要求テスト3: 軍事力が大きい国家ほど正規化値が高い。
    #[test]
    fn larger_military_yields_higher_normalized_value() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![state(1, 0, 1000), state(2, 1, 1000)]);
        let military_registry =
            military_with(vec![division(1, 0, 1, 10_000), division(2, 1, 2, 50_000)]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        let a = registry.get(CountryId(0)).unwrap();
        let b = registry.get(CountryId(1)).unwrap();
        assert!(b.military_normalized > a.military_normalized);
        assert_eq!(b.military_normalized, 100.0);
    }

    /// 要求テスト4: 既存軍事力計算(`calculate_country_total_power`)と結果が一致する。
    #[test]
    fn military_raw_matches_existing_calculate_country_total_power() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![state(1, 0, 1000), state(2, 1, 1000)]);
        let military_registry = military_with(vec![
            division(1, 0, 1, 10_000),
            division(2, 0, 1, 8_000),
            division(3, 1, 2, 20_000),
        ]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        for &id in &[0usize, 1usize] {
            let expected = crate::country::country_ai::calculate_country_total_power(
                CountryId(id),
                &military_registry,
                &state_registry,
            ) as f64;
            assert_eq!(registry.get(CountryId(id)).unwrap().military_raw, expected);
        }
    }

    /// 要求テスト5: `available_manpower`だけで順位が決まらない
    /// (募集可能人的資源が多くても、実際に配備された軍がなければ軍事力は0のまま)。
    #[test]
    fn available_manpower_alone_does_not_determine_military_power() {
        let country_registry = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(0),
                    available_manpower: 1_000_000,
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(1),
                    available_manpower: 100,
                    ..CountryData::default()
                },
            ],
        };
        let state_registry = states(vec![state(1, 0, 1000), state(2, 1, 1000)]);
        let military_registry = military_with(vec![division(1, 1, 2, 10_000)]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(registry.get(CountryId(0)).unwrap().military_raw, 0.0);
        assert!(registry.get(CountryId(1)).unwrap().military_raw > 0.0);
    }

    // ─── 経済力 ─────────────────────────────────────────────────────────────

    /// 要求テスト7: 生産建物を持つ国家の経済力が0より大きい。
    /// 要求テスト9: 非生産建物(Railway)を経済力へ含めない。
    #[test]
    fn economic_raw_counts_production_buildings_but_not_infrastructure() {
        let country_registry = countries(&[0, 1]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 2);
        let mut s1 = state(2, 1, 1000);
        s1.buildings.insert(BuildingType::Railway, 5);
        let state_registry = states(vec![s0, s1]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = building_registry();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert!(registry.get(CountryId(0)).unwrap().economic_raw > 0.0);
        assert_eq!(registry.get(CountryId(1)).unwrap().economic_raw, 0.0);
    }

    /// 要求テスト8: 生産建物が多い(レベルが高い)国家ほど経済力が高い。
    #[test]
    fn more_production_building_levels_yield_higher_economic_power() {
        let country_registry = countries(&[0, 1]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 1);
        let mut s1 = state(2, 1, 1000);
        s1.buildings.insert(BuildingType::Farm, 5);
        let state_registry = states(vec![s0, s1]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = building_registry();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert!(
            registry.get(CountryId(1)).unwrap().economic_raw
                > registry.get(CountryId(0)).unwrap().economic_raw
        );
    }

    /// 要求テスト10: 国庫残高だけを変えても経済力が変わらない。
    /// 要求テスト11: Stockpileだけを変えても経済力が変わらない。
    #[test]
    fn treasury_and_stockpile_do_not_affect_economic_power() {
        let mut country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 2);
        let state_registry = states(vec![s0]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = building_registry();

        let baseline = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        country_registry.countries[0].treasury = 999_999.0;
        country_registry.countries[0]
            .stockpile
            .add(ResourceType::Food, 999_999.0);

        let after = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(
            baseline.get(CountryId(0)).unwrap().economic_raw,
            after.get(CountryId(0)).unwrap().economic_raw
        );
    }

    /// 要求テスト12: CrystalMineを既存建物定義に従って評価する。
    #[test]
    fn crystal_mine_is_evaluated_from_its_building_definition() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::CrystalMine, 2);
        let state_registry = states(vec![s0]);
        let military_registry = MilitaryRegistry::default();
        let mut building_registry = building_registry();
        building_registry.definitions.insert(
            BuildingType::CrystalMine,
            building_def(
                BuildingType::CrystalMine,
                &[(ResourceType::RawMagicCrystal, 20.0)],
            ),
        );

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(registry.get(CountryId(0)).unwrap().economic_raw, 40.0);
    }

    /// 要求テスト12(補完): `Mine`は建物定義自体に産出量がないため、州の鉱床データから
    /// (discovered済み・MagicCrystal以外)算出する。
    #[test]
    fn mine_economic_power_comes_from_discovered_non_magic_deposits() {
        let country_registry = countries(&[0]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Mine, 3);
        s0.resource_deposits = vec![
            StateResourceDeposit {
                resource_type: ResourceType::Iron,
                base_output: 10.0,
                discovered: true,
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::Coal,
                base_output: 5.0,
                discovered: false, // 未発見: 含めない
                development_level: 1,
            },
            StateResourceDeposit {
                resource_type: ResourceType::MagicCrystal,
                base_output: 100.0,
                discovered: true, // MagicCrystalは専用CrystalMineの管轄: 含めない
                development_level: 1,
            },
        ];
        let state_registry = states(vec![s0]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = building_registry();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        // Iron(discovered)のみ: 10.0 * level(3) = 30.0
        assert_eq!(registry.get(CountryId(0)).unwrap().economic_raw, 30.0);
    }

    /// 要求テスト13: owner以外へ建物能力を加算しない(controllerが異なっていても、
    /// 経済力はownerへ計上される)。
    #[test]
    fn building_power_is_credited_to_owner_not_controller() {
        let country_registry = countries(&[0, 1]);
        let mut s0 = state(1, 0, 1000);
        s0.buildings.insert(BuildingType::Farm, 2);
        s0.controller_country = Some(CountryId(1)); // 占領中(実効支配は1だが所有権は0のまま)
        let state_registry = states(vec![s0]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = building_registry();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert!(registry.get(CountryId(0)).unwrap().economic_raw > 0.0);
        assert_eq!(registry.get(CountryId(1)).unwrap().economic_raw, 0.0);
    }

    // ─── 人口 ───────────────────────────────────────────────────────────────

    /// 要求テスト14: 所有州人口が正しく合計される。
    /// 要求テスト16: 複数州人口の合計。
    #[test]
    fn population_sums_across_multiple_owned_states() {
        let country_registry = countries(&[0]);
        let state_registry = states(vec![state(1, 0, 100_000), state(2, 0, 50_000)]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(
            registry.get(CountryId(0)).unwrap().population_raw,
            150_000.0
        );
    }

    /// 要求テスト15: controllerではなくownerへ計上される。
    #[test]
    fn population_is_credited_to_owner_not_controller() {
        let country_registry = countries(&[0, 1]);
        let mut s0 = state(1, 0, 100_000);
        s0.controller_country = Some(CountryId(1));
        let state_registry = states(vec![s0]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(
            registry.get(CountryId(0)).unwrap().population_raw,
            100_000.0
        );
        assert_eq!(registry.get(CountryId(1)).unwrap().population_raw, 0.0);
    }

    /// 要求テスト17: 州を持たない国家は人口0。
    #[test]
    fn stateless_country_has_zero_population() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![state(1, 0, 100_000)]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(registry.get(CountryId(1)).unwrap().population_raw, 0.0);
    }

    /// 要求テスト18: 不正な人口値(u64のため負値は型上不可能だが、極端な巨大値でも)
    /// NaN・panicを発生させない。
    #[test]
    fn extreme_population_values_do_not_panic_or_produce_nan() {
        let country_registry = countries(&[0]);
        let state_registry = states(vec![state(1, 0, u64::MAX)]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        let a = registry.get(CountryId(0)).unwrap();
        assert!(a.population_raw.is_finite());
        assert!(a.population_normalized.is_finite());
        assert!(a.total_score.is_finite());
    }

    // ─── 正規化・総合力 ─────────────────────────────────────────────────────

    /// 要求テスト19: 世界最大値を100へ正規化。
    #[test]
    fn world_max_normalizes_to_100() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![state(1, 0, 100_000), state(2, 1, 50_000)]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(
            registry.get(CountryId(0)).unwrap().population_normalized,
            100.0
        );
    }

    /// 要求テスト20: 全国家0なら全正規化値0。
    #[test]
    fn all_zero_yields_all_normalized_zero() {
        let country_registry = countries(&[0, 1]);
        let state_registry = states(vec![]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        for id in [CountryId(0), CountryId(1)] {
            let a = registry.get(id).unwrap();
            assert_eq!(a.military_normalized, 0.0);
            assert_eq!(a.economic_normalized, 0.0);
            assert_eq!(a.population_normalized, 0.0);
            assert_eq!(a.total_score, 0.0);
        }
    }

    /// 要求テスト21: 50/30/20の重みが正しい。
    #[test]
    fn total_score_uses_50_30_20_weights() {
        // 2か国、片方だけ全要素で世界最大値(=正規化値100)を独占する状況を作る。
        let country_registry = countries(&[0, 1]);
        let mut s0 = state(1, 0, 100_000);
        s0.buildings.insert(BuildingType::Farm, 5);
        let s1 = state(2, 1, 1);
        let state_registry = states(vec![s0, s1]);
        let military_registry =
            military_with(vec![division(1, 0, 1, 10_000), division(2, 1, 2, 1)]);
        let building_registry = building_registry();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        let a = registry.get(CountryId(0)).unwrap();
        let expected = a.military_normalized as f64 * 0.50
            + a.economic_normalized as f64 * 0.30
            + a.population_normalized as f64 * 0.20;
        assert!((a.total_score as f64 - expected).abs() < 1e-6);
    }

    /// 要求テスト22: 総合力が0〜100に収まる。
    /// 要求テスト23: NaN・無限値を生成しない。
    #[test]
    fn total_score_is_always_within_bounds_and_finite() {
        let country_registry = countries(&[0, 1, 2]);
        let state_registry = states(vec![state(1, 0, 1_000_000), state(2, 1, 1), state(3, 2, 0)]);
        let military_registry = military_with(vec![division(1, 0, 1, 500_000)]);
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        for id in [CountryId(0), CountryId(1), CountryId(2)] {
            let a = registry.get(id).unwrap();
            assert!(a.total_score.is_finite());
            assert!((0.0..=100.0).contains(&a.total_score));
        }
    }

    /// 要求テスト24: 同じ入力から同じ結果(決定論)。
    #[test]
    fn same_input_produces_same_result() {
        let country_registry = countries(&[0, 1, 2]);
        let state_registry = states(vec![state(1, 0, 10_000), state(2, 1, 20_000)]);
        let military_registry = military_with(vec![division(1, 0, 1, 5_000)]);
        let building_registry = building_registry();

        let a = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );
        let b = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        for id in [CountryId(0), CountryId(1), CountryId(2)] {
            assert_eq!(a.get(id), b.get(id));
        }
        assert_eq!(a.ordered_country_ids(), b.ordered_country_ids());
    }

    /// 要求テスト25: HashMap挿入順を変えても同じ結果。
    #[test]
    fn insertion_order_does_not_affect_result() {
        let country_registry_a = countries(&[0, 1, 2]);
        let country_registry_b = countries(&[2, 1, 0]);
        let state_registry = states(vec![
            state(1, 2, 30_000),
            state(2, 0, 10_000),
            state(3, 1, 20_000),
        ]);
        let military_registry =
            military_with(vec![division(3, 1, 3, 5_000), division(1, 0, 2, 2_000)]);
        let building_registry = building_registry();

        let a = evaluate_country_power(
            &country_registry_a,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );
        let b = evaluate_country_power(
            &country_registry_b,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        for id in [CountryId(0), CountryId(1), CountryId(2)] {
            assert_eq!(a.get(id), b.get(id));
        }
    }

    // ─── ランク ─────────────────────────────────────────────────────────────

    /// 要求テスト26: 国家数0。
    #[test]
    fn zero_countries_yields_empty_registry() {
        let country_registry = CountryRegistry { countries: vec![] };
        let state_registry = states(vec![]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(registry.country_count(), 0);
        assert!(registry.ordered_country_ids().is_empty());
    }

    /// 要求テスト27-32: 国家数ごとの大国/地域大国/小国の人数。
    #[test]
    fn tier_counts_match_the_specified_table() {
        let cases: [(usize, usize, usize, usize); 6] = [
            (1, 1, 0, 0),
            (2, 1, 1, 0),
            (7, 2, 3, 2),
            (10, 2, 3, 5),
            (50, 8, 15, 27),
            (100, 8, 30, 62),
        ];
        for (n, great, regional, minor) in cases {
            let (g, r) = compute_tier_counts(n);
            assert_eq!(g, great, "n={n}: great_power_count mismatch");
            assert_eq!(r, regional, "n={n}: regional_power_count mismatch");
            assert_eq!(n - g - r, minor, "n={n}: minor_power_count mismatch");
        }
    }

    /// 要求テスト33: 同点時に軍事力→経済力→人口→CountryId順で決定する。
    #[test]
    fn ties_are_broken_by_military_then_economic_then_population_then_country_id() {
        // 全国家を完全に同点(総合力・3要素すべて0)にし、CountryId昇順のみで決まることを確認する。
        let country_registry = countries(&[5, 2, 8]);
        let state_registry = states(vec![]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(
            registry.ordered_country_ids(),
            &[CountryId(2), CountryId(5), CountryId(8)]
        );
    }

    /// 要求テスト34: 世界順位が1からNまで重複しない。
    /// 要求テスト36: ordered listが順位順。
    #[test]
    fn world_rank_is_unique_1_to_n_and_matches_ordered_list() {
        let country_registry = countries(&[0, 1, 2, 3]);
        let state_registry = states(vec![
            state(1, 0, 40_000),
            state(2, 1, 30_000),
            state(3, 2, 20_000),
            state(4, 3, 10_000),
        ]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        let mut ranks: Vec<usize> = registry
            .ordered_country_ids()
            .iter()
            .map(|&id| registry.get(id).unwrap().world_rank)
            .collect();
        ranks.sort_unstable();
        assert_eq!(ranks, vec![1, 2, 3, 4]);

        for (idx, &id) in registry.ordered_country_ids().iter().enumerate() {
            assert_eq!(registry.get(id).unwrap().world_rank, idx + 1);
        }
    }

    /// 要求テスト35: RegistryのO(1)取得(存在しないIDは`None`、panicしない)。
    #[test]
    fn get_is_safe_for_nonexistent_country_id() {
        let country_registry = countries(&[0]);
        let state_registry = states(vec![]);
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert!(registry.get(CountryId(999)).is_none());
    }

    // ─── エラー処理 ─────────────────────────────────────────────────────────

    /// 要求: 存在しないCountryIdを参照するState(ownerが実在しない国家)があっても、
    /// 幽霊の評価エントリを作らずpanicもしない。
    #[test]
    fn dangling_owner_reference_does_not_create_phantom_assessment_or_panic() {
        let country_registry = countries(&[0]);
        let state_registry = states(vec![state(1, 999, 50_000)]); // owner 999 は存在しない
        let military_registry = MilitaryRegistry::default();
        let building_registry = BuildingRegistry::default();

        let registry = evaluate_country_power(
            &country_registry,
            &state_registry,
            &military_registry,
            &building_registry,
            "1800/01/01".to_string(),
        );

        assert_eq!(registry.country_count(), 1);
        assert!(registry.get(CountryId(999)).is_none());
        assert_eq!(registry.get(CountryId(0)).unwrap().population_raw, 0.0);
    }
}
