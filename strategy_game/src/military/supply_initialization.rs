//! P26-004-PRE-005: MilitarySuppliesの初期ブリッジ備蓄付与。
//!
//! New Game開始時に、初回月次生産までをつなぐMilitarySuppliesのブリッジ備蓄を各国家へ
//! 一度限り付与し、旧Saveへは同じ計算式で意味的migrationとして同じ付与を適用する
//! (`verification_logs/phase-26/p26-004-pre-005/`)。P26-004本体(日次消費・Division配分・
//! 補給充足率)はここでは実装しない――将来P26-004が再利用できるよう、正規の日次需要式
//! だけをこのモジュールで確立する。
//!
//! ## 設計の要点
//!
//! * 日次需要は`daily_demand = definition.supply_usage × current_manpower / max_manpower ×
//!   activity_multiplier`(本タスクの`activity_multiplier`は常に`1.0`、spec §5)。
//! * ブリッジ必要量は`daily_demand_units × (次回月次生産までの消費イベント数 + 1日の予備)`
//!   (spec §6)。New Gameの現行データ(1800年1月1日開始、満編制Infantry1個師団)では
//!   30イベント+1日予備=31日分=31.0になる(`src/app/time.rs::advance_game_date`の
//!   制御フローから導出、report参照)。
//! * 旧Saveのtop-up式は`grant = max(0, required_bridge_units - current_stockpile_units)`
//!   (spec §8.2)――無条件加算はしない、既存備蓄を減らさない。
//! * New Game・旧Save双方が、この1モジュールが提供する同じ純粋関数
//!   (`apply_military_supplies_bridge_grant_for_country`)を呼ぶ。二重付与は国家単位の
//!   一度限りマーカー(`CountryData::military_supplies_initial_grant_applied`)で防ぐ。

use crate::app::time::GameDate;
use crate::common::{CountryId, DivisionDefinitionId, DivisionId};
use crate::country::CountryData;
use crate::economy::resources::{
    MilitarySuppliesUnitsError, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use crate::military::data::{Division, DivisionDefinition, DivisionStatus, MilitaryRegistry};
use bevy::prelude::*;
use std::collections::HashMap;

// ─── Division日次需要(spec §5) ────────────────────────────────────────────

/// P26-004本体が実装するまで固定される活動倍率(spec §5「本タスクの活動倍率」)。
/// 待機・移動・戦闘等による倍率差はまだ導入しない。将来P26-004がDivision状態別の
/// 倍率を導入する際は、この定数の代わりに`DivisionStatus`から倍率を導出する関数へ
/// 差し替える想定(`division_daily_demand_units`の呼び出し側だけを変更すればよい)。
pub const BASELINE_ACTIVITY_MULTIPLIER: f64 = 1.0;

/// Division需要計算が失敗し得る理由。不正なDivisionを黙って需要0として扱わない
/// (spec §5.2)。生存判定(`is_living`)で除外される「死亡」Divisionはここでは
/// 対象外――正規のライフサイクル終端であり不正状態ではないため。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SupplyDemandError {
    /// `current_manpower > max_manpower`という不正状態。
    ManpowerExceedsMax {
        division_id: DivisionId,
        manpower: u64,
        max_manpower: u64,
    },
    /// `max_manpower == 0`なのに`current_manpower > 0`という不正状態。
    MaxManpowerZeroWithPositiveManpower {
        division_id: DivisionId,
        manpower: u64,
    },
    /// `definitions`に存在しない`DivisionDefinitionId`を参照している。
    UnknownDefinition {
        division_id: DivisionId,
        def_id: DivisionDefinitionId,
    },
    /// `supply_usage`がNaN・Infinity・負数・値域外など、正規unitsへ量子化できない。
    InvalidSupplyUsage {
        division_id: DivisionId,
        supply_usage: f32,
    },
    /// units上のchecked演算が(理論上到達不能だが)overflowした。
    Overflow,
}

/// 生存Divisionかどうか(既存コードベース全体の規約、`military::manpower::is_living`と
/// 同じ判定式を複製する――`manpower > 0 && status != Destroyed`)。死亡Divisionは
/// 需要0として自然に除外する(不正状態としては扱わない、正規のライフサイクル終端)。
fn is_living(division: &Division) -> bool {
    division.manpower > 0 && division.status != DivisionStatus::Destroyed
}

/// `usage_units × manpower / max_manpower`のfloor除算を、checked整数演算だけで行う
/// 内部ヘルパー(`division_daily_demand_units`から分離し、overflow境界を直接単体
/// テストできるようにする)。`max_manpower > 0`は呼び出し側が保証する。
///
/// 注記: `usage_units`は`military_supplies_value_to_units`が返す`u64`であり
/// `MILITARY_SUPPLIES_MAX_UNITS`(10^15)以下、`manpower`も`u64`であるため、
/// `u128`へ拡張した積は数学的に`u128::MAX`を超え得ない(`u64::MAX^2 < u128::MAX`)。
/// したがってこの`checked_mul`が実際に`None`を返すことは現在の型境界では起こり得ない
/// 防御的分岐だが、将来型が拡張された場合に備えてchecked演算のまま維持する。
fn checked_scaled_demand_units(
    usage_units: u64,
    manpower: u64,
    max_manpower: u64,
) -> Result<u128, SupplyDemandError> {
    let numerator = (usage_units as u128)
        .checked_mul(manpower as u128)
        .ok_or(SupplyDemandError::Overflow)?;
    Ok(numerator / (max_manpower as u128))
}

/// 1個Divisionの日次需要unitsを計算する(spec §5.1の正規式)。
///
/// ```text
/// usage_units = Q(definition.supply_usage × activity_multiplier)
/// demand_units = floor(usage_units × current_manpower / max_manpower)
/// ```
///
/// `Q`はP26-002の`military_supplies_value_to_units`(MilitarySupplies正規変換関数)。
/// `u128`の整数除算は既定でfloor除算のため、上記の`floor`が自然に成立する。
///
/// 境界仕様(spec §5.2):
/// * `current_manpower == 0` → 需要0(`max_manpower`の値に関わらず)。
/// * `max_manpower == 0 && current_manpower == 0` → 需要0。
/// * `max_manpower == 0 && current_manpower > 0` → `MaxManpowerZeroWithPositiveManpower`。
/// * `current_manpower > max_manpower` → `ManpowerExceedsMax`。
/// * 不明`definition` → `UnknownDefinition`。
/// * `supply_usage`がNaN/Infinity/負数/範囲外 → `InvalidSupplyUsage`。
pub fn division_daily_demand_units(
    division: &Division,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
) -> Result<u128, SupplyDemandError> {
    let def = definitions
        .get(&division.def_id)
        .ok_or(SupplyDemandError::UnknownDefinition {
            division_id: division.id,
            def_id: division.def_id,
        })?;

    if division.max_manpower == 0 {
        if division.manpower == 0 {
            return Ok(0);
        }
        return Err(SupplyDemandError::MaxManpowerZeroWithPositiveManpower {
            division_id: division.id,
            manpower: division.manpower,
        });
    }
    if division.manpower > division.max_manpower {
        return Err(SupplyDemandError::ManpowerExceedsMax {
            division_id: division.id,
            manpower: division.manpower,
            max_manpower: division.max_manpower,
        });
    }
    if division.manpower == 0 {
        return Ok(0);
    }

    let usage_scaled = (def.supply_usage as f64) * BASELINE_ACTIVITY_MULTIPLIER;
    let usage_units = military_supplies_value_to_units(usage_scaled).map_err(|_| {
        SupplyDemandError::InvalidSupplyUsage {
            division_id: division.id,
            supply_usage: def.supply_usage,
        }
    })?;

    checked_scaled_demand_units(usage_units, division.manpower, division.max_manpower)
}

/// `total`へ`addition`をchecked加算する内部ヘルパー(overflow境界を直接単体テスト
/// できるようにする)。
fn checked_add_demand(total: u128, addition: u128) -> Result<u128, SupplyDemandError> {
    total.checked_add(addition).ok_or(SupplyDemandError::Overflow)
}

/// 指定した国家が保有する全ての生存Divisionの日次需要units合計を計算する(spec §5)。
/// `HashMap`(`military_registry.divisions`)の走査順に依存しないよう、`DivisionId`
/// 昇順へソートしてから合算する(合計は可換な加算のみのため走査順自体は結果を
/// 変えないが、決定的な処理順を明示するために固定する)。
pub fn country_daily_demand_units(
    country_id: CountryId,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
) -> Result<u128, SupplyDemandError> {
    let mut ids: Vec<DivisionId> = divisions
        .values()
        .filter(|d| d.owner == country_id)
        .map(|d| d.id)
        .collect();
    ids.sort_by_key(|id| id.0);

    let mut total: u128 = 0;
    for id in ids {
        let division = &divisions[&id];
        if !is_living(division) {
            continue;
        }
        let demand = division_daily_demand_units(division, definitions)?;
        total = checked_add_demand(total, demand)?;
    }
    Ok(total)
}

// ─── カレンダー(spec §6.1) ────────────────────────────────────────────────

/// カレンダー状態が不正で、次回月次生産までの消費イベント数を一意に復元できない
/// 場合のエラー(spec §17 BLOCKED条件に対応する防御的分岐、通常のゲーム状態
/// [有効な`GameDate`]では到達しない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarError {
    /// `month`が1〜12の範囲外。
    InvalidMonth { month: u8 },
    /// `day`が0、または`days_in_month`を超える。
    DayOutOfRange { day: u8, days_in_month: u8 },
}

/// `month`(1〜12)の日数を返す(`app::time::GameDate::days_in_month`と同じ暦規則、
/// うるう年は考慮しない簡易暦)。
pub fn days_in_month(month: u8) -> Result<u8, CalendarError> {
    GameDate::days_in_month(month).ok_or(CalendarError::InvalidMonth { month })
}

/// 現在日から次回月次生産(月境界の`MonthChangedMessage`)までに確実に発生する
/// `DayChangedMessage`の回数を計算する(spec §6.1)。
///
/// `src/app/time.rs::advance_game_date`を実コードで監査した結果(report参照):
/// * 現在日(`day`)自体は消費回数へ含めない――このSystem実行時点で既に確定済みの
///   日として扱う。
/// * `day+1`から`days_in_month`まで(`days_in_month - day`回)は確実に月境界より前に
///   発行される`DayChangedMessage`である。
/// * `days_in_month`の次の1回(月が繰り上がる当日)は、その同一イテレーション内で
///   `MonthChangedMessage`も発行されるため、その日の消費が生産の前か後かは処理順序に
///   依存し確定しない「境界日」である――この1日分は本関数の戻り値には含めず、
///   呼び出し側(`compute_required_bridge_units`)が別途「1日分の予備」として加算する。
///
/// 例: New Game開始日(`day=1`, 1月=31日) → `31 - 1 = 30`。
pub fn events_before_next_monthly_production(
    day: u8,
    days_in_month: u8,
) -> Result<u32, CalendarError> {
    if day == 0 || day > days_in_month {
        return Err(CalendarError::DayOutOfRange { day, days_in_month });
    }
    Ok((days_in_month - day) as u32)
}

// ─── ブリッジ必要量(spec §6) ──────────────────────────────────────────────

/// ブリッジ必要量(units)を計算する。
///
/// ```text
/// bridge_units = daily_demand_units × (events_before_next_production + 1日の予備)
/// ```
///
/// New Gameの現行データ(`daily_demand = 1.0`, `events = 30`)では
/// `1_000_000 × (30 + 1) = 31_000_000`units(=31.0)になる。
pub fn compute_required_bridge_units(
    daily_demand_units: u128,
    events_before_next_production: u32,
) -> Result<u128, SupplyDemandError> {
    let total_days = (events_before_next_production as u128)
        .checked_add(1)
        .ok_or(SupplyDemandError::Overflow)?;
    daily_demand_units
        .checked_mul(total_days)
        .ok_or(SupplyDemandError::Overflow)
}

// ─── 1国家分の原子的付与(spec §8〜10) ─────────────────────────────────────

/// `apply_military_supplies_bridge_grant_for_country`が失敗し得る理由。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BridgeGrantError {
    Demand(SupplyDemandError),
    Calendar(CalendarError),
    /// 現在備蓄が不正(NaN・負数等)で正規unitsへ変換できない。
    InvalidCurrentStockpile,
    /// 必要bridge unitsまたはgrant unitsが`u64`域を超えるか、往復変換に失敗した。
    InvalidRequiredOrGrantValue,
    /// 備蓄への実際のcommit(`try_grant_military_supplies`)が失敗した。
    Apply(MilitarySuppliesUnitsError),
}

impl From<SupplyDemandError> for BridgeGrantError {
    fn from(e: SupplyDemandError) -> Self {
        BridgeGrantError::Demand(e)
    }
}

impl From<CalendarError> for BridgeGrantError {
    fn from(e: CalendarError) -> Self {
        BridgeGrantError::Calendar(e)
    }
}

/// `apply_military_supplies_bridge_grant_for_country`の成功時の結果分類
/// (呼び出し側のログ・テスト検証用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGrantOutcome {
    /// マーカーが既にtrueだったため何もしなかった(冪等性、spec §8.1)。
    AlreadyGranted,
    /// 消滅国家のため需要計算をスキップし、grant 0・markerをtrueにした(spec §8.3)。
    EliminatedCountrySkipped,
    /// 必要量が既存備蓄以下だった(Divisionを持たない国家を含む)ため、備蓄は変更せず
    /// markerだけtrueにした。
    GrantedZero,
    /// 実際に備蓄へunits分を加算し、markerをtrueにした。
    Granted { units: u128 },
}

/// 1国家分のMilitarySuppliesブリッジ備蓄付与を原子的に実行する(spec §10)。
///
/// 手順(すべて成功した場合のみ`country.stockpile`・
/// `country.military_supplies_initial_grant_applied`を変更する。途中で失敗した場合は
/// どちらも一切変更しない):
/// 1. マーカーが既にtrueなら即座に`AlreadyGranted`(no-op)。
/// 2. 消滅国家(`!country.is_active()`)なら需要計算を行わず`EliminatedCountrySkipped`
///    としてmarkerをtrueにする(spec §8.3)。
/// 3. Division需要を検証・合算する(`country_daily_demand_units`)。
/// 4. カレンダーから必要bridge unitsを計算する(`compute_required_bridge_units`)。
/// 5. 現在備蓄unitsを検証する。
/// 6. `grant_units = max(0, required - current)`を計算する(spec §8.2)。
/// 7. `grant_units == 0`なら備蓄を変更せずmarkerだけtrueにする。
/// 8. `grant_units > 0`ならunits→value往復検証の後、`try_grant_military_supplies`で
///    1回だけcommitし、成功した場合のみmarkerをtrueにする。
pub fn apply_military_supplies_bridge_grant_for_country(
    country: &mut CountryData,
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
    current_day: u8,
    current_month: u8,
) -> Result<BridgeGrantOutcome, BridgeGrantError> {
    if country.military_supplies_initial_grant_applied {
        return Ok(BridgeGrantOutcome::AlreadyGranted);
    }

    if !country.is_active() {
        country.military_supplies_initial_grant_applied = true;
        return Ok(BridgeGrantOutcome::EliminatedCountrySkipped);
    }

    let daily_demand_units = country_daily_demand_units(country.id, divisions, definitions)?;

    let month_length = days_in_month(current_month)?;
    let events = events_before_next_monthly_production(current_day, month_length)?;
    let required_bridge_units = compute_required_bridge_units(daily_demand_units, events)?;

    let current_value = country.stockpile.get(ResourceType::MilitarySupplies);
    let current_units = military_supplies_value_to_units(current_value)
        .map_err(|_| BridgeGrantError::InvalidCurrentStockpile)?;

    // grant_units = max(0, required - current)(spec §8.2の定義そのもの)。
    // `current > required`(既に十分な備蓄がある)は不正状態でも到達不能な
    // overflowでもなく、"既存備蓄を減らさない"という正規の分岐であるため、
    // `saturating_sub`が意味的に正しい実装であり、`checked_sub().unwrap_or(0)`
    // という手書きの等価コードよりこちらを使うべきだとclippyも指摘する
    // (`clippy::manual_saturating_arithmetic`)。ここでの用途は「本当は
    // checked演算でエラー検出すべき箇所をsaturatingで握り潰す」というspec §5.1の
    // 禁止パターンとは異なる――§5.1が禁止するのはDivision需要のunits変換・
    // checked_add/checked_mul(値域超過やoverflowが本当に異常事態でErrへ落とすべき
    // 箇所)であり、本行はそれらのchecked演算の"結果"を使ってmax(0, a-b)という
    // 別の(下限0でクランプすることが仕様そのものである)計算をしているだけである。
    let grant_units = required_bridge_units.saturating_sub(current_units as u128);

    if grant_units == 0 {
        country.military_supplies_initial_grant_applied = true;
        return Ok(BridgeGrantOutcome::GrantedZero);
    }

    let grant_units_u64 =
        u64::try_from(grant_units).map_err(|_| BridgeGrantError::InvalidRequiredOrGrantValue)?;
    let grant_value = military_supplies_units_to_value(grant_units_u64);
    match military_supplies_value_to_units(grant_value) {
        Ok(round_trip) if round_trip == grant_units_u64 => {}
        _ => return Err(BridgeGrantError::InvalidRequiredOrGrantValue),
    }

    country
        .stockpile
        .try_grant_military_supplies(grant_value)
        .map_err(BridgeGrantError::Apply)?;

    country.military_supplies_initial_grant_applied = true;
    Ok(BridgeGrantOutcome::Granted { units: grant_units })
}

// ─── New Game System(spec §7) ─────────────────────────────────────────────

/// New Game初期化時に、全国家へMilitarySuppliesブリッジ備蓄を一度限り付与する
/// Bevy System(spec §7)。
///
/// `src/app/loader.rs`の`OnEnter(GameState::Playing)`チェーンで
/// `spawn_debug_divisions`の直後・`rebuild_country_power_on_enter_playing`の前に置く:
/// 全国家の初期Divisionが生成・確定した後に需要を計算し、国力再評価より前に備蓄へ
/// 反映する(国力計算自体は変更しない)。`spawn_debug_divisions`と同じ
/// `PlayingEntryMode::NewGame`ガードを共有するため、Loaded Game進入では実行しない
/// (Loaded Game側の意味的migrationは`save::apply::prepare_load`が別途、
/// `apply_military_supplies_bridge_migration`経由で担当する)。`CountryId`昇順で
/// 決定的に処理する(`HashMap`走査順に依存しない)。
pub fn grant_initial_military_supplies_bridge(
    mut country_registry: ResMut<crate::country::CountryRegistry>,
    military_registry: Res<MilitaryRegistry>,
    game_date: Res<GameDate>,
    entry_mode: Res<crate::app::game_state::PlayingEntryMode>,
) {
    if *entry_mode != crate::app::game_state::PlayingEntryMode::NewGame {
        return;
    }

    let mut country_ids: Vec<CountryId> =
        country_registry.countries.iter().map(|c| c.id).collect();
    country_ids.sort_by_key(|id| id.0);

    for id in country_ids {
        let Some(country) = country_registry.get_mut(id) else {
            continue;
        };
        if let Err(err) = apply_military_supplies_bridge_grant_for_country(
            country,
            &military_registry.divisions,
            &military_registry.definitions,
            game_date.day,
            game_date.month,
        ) {
            warn!(
                "[P26-004-PRE-005] initial military supplies bridge grant failed for {:?}: \
                 {:?} (stockpile and marker left unchanged, will retry on the next New Game \
                 entry)",
                id, err
            );
        }
    }
}

// ─── 旧Save意味的migration(spec §8) ───────────────────────────────────────

/// 旧Save(`src/save/apply.rs::prepare_load`)へMilitarySuppliesブリッジ備蓄の
/// 意味的migrationを適用する(spec §8)。New Gameと全く同じ
/// `apply_military_supplies_bridge_grant_for_country`を、`countries`の`CountryId`
/// 昇順で1か国ずつ呼ぶ。
///
/// 原子性境界は国家単位: 1か国の失敗(不正Division・不正備蓄等)は`warn!`で記録して
/// 処理を続け、他国のmigrationを妨げない(`supply_chain_migration`が採用した
/// 「世界全体を一括validate→apply」する境界とは意図的に異なる――各国家のgrantが
/// 完全に独立しており、1か国の異常値が他国の意味的migrationを妨げるべきではない
/// ため。マーカーが立たなかった国家は、次回ロード時に自動的に再試行される)。
pub fn apply_military_supplies_bridge_migration(
    countries: &mut [CountryData],
    divisions: &HashMap<DivisionId, Division>,
    definitions: &HashMap<DivisionDefinitionId, DivisionDefinition>,
    save_day: u8,
    save_month: u8,
) {
    let mut ids: Vec<CountryId> = countries.iter().map(|c| c.id).collect();
    ids.sort_by_key(|id| id.0);

    for id in ids {
        let Some(country) = countries.iter_mut().find(|c| c.id == id) else {
            continue;
        };
        if let Err(err) = apply_military_supplies_bridge_grant_for_country(
            country,
            divisions,
            definitions,
            save_day,
            save_month,
        ) {
            warn!(
                "[P26-004-PRE-005] bridge grant migration failed for country {:?}: {:?} \
                 (stockpile and marker left unchanged for this country, other countries are \
                 unaffected, will retry on the next load)",
                id, err
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::StateId;
    use crate::country::CountryStatus;
    use crate::economy::resources::CountryStockpile;
    use crate::military::data::{DivisionSize, DivisionType};

    // ─── テスト用フィクスチャ ─────────────────────────────────────────

    fn infantry_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            id: DivisionDefinitionId(id),
            name: "Test Infantry".to_string(),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            required_manpower: 10_000,
            required_equipment: 100.0,
            recruitment_days: 10,
            movement_speed: 1.0,
            attack: 10.0,
            defense: 10.0,
            breakthrough: 1.0,
            organization: 50.0,
            morale: 50.0,
            supply_usage: 1.0,
            maintenance_cost: 1.0,
        }
    }

    fn artillery_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            supply_usage: 3.0,
            division_type: DivisionType::Artillery,
            ..infantry_def(id)
        }
    }

    fn mage_def(id: usize) -> DivisionDefinition {
        DivisionDefinition {
            supply_usage: 2.0,
            division_type: DivisionType::Mage,
            ..infantry_def(id)
        }
    }

    fn division(
        div_id: usize,
        owner: usize,
        def_id: usize,
        manpower: u64,
        max_manpower: u64,
    ) -> Division {
        Division {
            id: DivisionId(div_id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower,
            max_manpower,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 50.0,
            max_morale: 50.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: DivisionDefinitionId(def_id),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    fn defs(list: Vec<DivisionDefinition>) -> HashMap<DivisionDefinitionId, DivisionDefinition> {
        list.into_iter().map(|d| (d.id, d)).collect()
    }

    fn divs(list: Vec<Division>) -> HashMap<DivisionId, Division> {
        list.into_iter().map(|d| (d.id, d)).collect()
    }

    fn test_country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            ..CountryData::default()
        }
    }

    // ─── Division需要(spec §13) ──────────────────────────────────────

    #[test]
    fn full_strength_infantry_division_demands_exactly_one_million_units() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 1_000_000);
    }

    #[test]
    fn half_manpower_infantry_demands_half() {
        let d = division(0, 0, 0, 5_000, 10_000);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 500_000);
    }

    #[test]
    fn zero_manpower_division_demands_zero() {
        let d = division(0, 0, 0, 0, 10_000);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 0);
    }

    #[test]
    fn full_strength_artillery_demands_three_million_units() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![artillery_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 3_000_000);
    }

    #[test]
    fn full_strength_mage_demands_two_million_units() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![mage_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 2_000_000);
    }

    #[test]
    fn multiple_branches_sum_correctly_for_one_country() {
        let definitions = defs(vec![infantry_def(0), artillery_def(1), mage_def(2)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000), // 1_000_000
            division(1, 0, 1, 10_000, 10_000), // 3_000_000
            division(2, 0, 2, 10_000, 10_000), // 2_000_000
        ]);
        let total = country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        assert_eq!(total, 6_000_000);
    }

    #[test]
    fn multiple_divisions_of_the_same_country_sum() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 0, 10_000, 10_000),
        ]);
        let total = country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        assert_eq!(total, 2_000_000);
    }

    #[test]
    fn countries_are_isolated_from_each_other() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 1, 0, 10_000, 10_000),
            division(2, 1, 0, 10_000, 10_000),
        ]);
        assert_eq!(
            country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap(),
            1_000_000
        );
        assert_eq!(
            country_daily_demand_units(CountryId(1), &divisions, &definitions).unwrap(),
            2_000_000
        );
    }

    #[test]
    fn division_insertion_order_does_not_change_the_total() {
        let definitions = defs(vec![infantry_def(0), artillery_def(1)]);
        let list_a = vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 4_000, 10_000),
            division(2, 0, 0, 7_000, 10_000),
        ];
        let mut list_b = list_a.clone();
        list_b.reverse();

        let total_a =
            country_daily_demand_units(CountryId(0), &divs(list_a), &definitions).unwrap();
        let total_b =
            country_daily_demand_units(CountryId(0), &divs(list_b), &definitions).unwrap();
        assert_eq!(total_a, total_b);
    }

    #[test]
    fn definition_insertion_order_does_not_change_the_total() {
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 0, 1, 10_000, 10_000),
        ]);
        let defs_a = defs(vec![infantry_def(0), artillery_def(1)]);
        let defs_b = defs(vec![artillery_def(1), infantry_def(0)]);

        let total_a = country_daily_demand_units(CountryId(0), &divisions, &defs_a).unwrap();
        let total_b = country_daily_demand_units(CountryId(0), &divisions, &defs_b).unwrap();
        assert_eq!(total_a, total_b);
    }

    #[test]
    fn country_iteration_order_is_independent_via_direct_filtering() {
        // country_daily_demand_unitsは呼び出しごとに対象CountryIdだけを`divisions`
        // からフィルタするため、`divisions`自体へ他国のエントリを先に挿入しても
        // 後に挿入しても結果は変わらない。
        let definitions = defs(vec![infantry_def(0)]);
        let mut list_a = vec![
            division(0, 5, 0, 10_000, 10_000),
            division(1, 0, 0, 10_000, 10_000),
        ];
        let mut list_b = list_a.clone();
        list_b.reverse();
        list_a.push(division(2, 9, 0, 10_000, 10_000));
        list_b.insert(0, division(2, 9, 0, 10_000, 10_000));

        let total_a =
            country_daily_demand_units(CountryId(0), &divs(list_a), &definitions).unwrap();
        let total_b =
            country_daily_demand_units(CountryId(0), &divs(list_b), &definitions).unwrap();
        assert_eq!(total_a, 1_000_000);
        assert_eq!(total_b, 1_000_000);
    }

    #[test]
    fn max_manpower_zero_and_manpower_zero_is_valid_zero_demand() {
        let d = division(0, 0, 0, 0, 0);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(division_daily_demand_units(&d, &definitions).unwrap(), 0);
    }

    #[test]
    fn max_manpower_zero_with_positive_manpower_is_an_explicit_error() {
        let d = division(0, 0, 0, 1, 0);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::MaxManpowerZeroWithPositiveManpower {
                division_id: DivisionId(0),
                manpower: 1,
            })
        );
    }

    #[test]
    fn manpower_exceeding_max_manpower_is_an_explicit_error() {
        let d = division(0, 0, 0, 11_000, 10_000);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::ManpowerExceedsMax {
                division_id: DivisionId(0),
                manpower: 11_000,
                max_manpower: 10_000,
            })
        );
    }

    #[test]
    fn unknown_definition_is_an_explicit_error() {
        let d = division(0, 0, 99, 10_000, 10_000);
        let definitions = defs(vec![infantry_def(0)]);
        assert_eq!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::UnknownDefinition {
                division_id: DivisionId(0),
                def_id: DivisionDefinitionId(99),
            })
        );
    }

    #[test]
    fn nan_supply_usage_is_an_explicit_error() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![DivisionDefinition {
            supply_usage: f32::NAN,
            ..infantry_def(0)
        }]);
        assert!(matches!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::InvalidSupplyUsage { .. })
        ));
    }

    #[test]
    fn infinite_supply_usage_is_an_explicit_error() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![DivisionDefinition {
            supply_usage: f32::INFINITY,
            ..infantry_def(0)
        }]);
        assert!(matches!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::InvalidSupplyUsage { .. })
        ));
    }

    #[test]
    fn negative_supply_usage_is_an_explicit_error() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![DivisionDefinition {
            supply_usage: -1.0,
            ..infantry_def(0)
        }]);
        assert!(matches!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::InvalidSupplyUsage { .. })
        ));
    }

    #[test]
    fn out_of_range_supply_usage_is_an_explicit_error() {
        let d = division(0, 0, 0, 10_000, 10_000);
        let definitions = defs(vec![DivisionDefinition {
            supply_usage: f32::MAX,
            ..infantry_def(0)
        }]);
        assert!(matches!(
            division_daily_demand_units(&d, &definitions),
            Err(SupplyDemandError::InvalidSupplyUsage { .. })
        ));
    }

    #[test]
    fn dead_divisions_are_excluded_without_error() {
        let definitions = defs(vec![infantry_def(0)]);
        let mut destroyed = division(0, 0, 0, 10_000, 10_000);
        destroyed.status = DivisionStatus::Destroyed;
        let divisions = divs(vec![destroyed, division(1, 0, 0, 10_000, 10_000)]);
        let total = country_daily_demand_units(CountryId(0), &divisions, &definitions).unwrap();
        assert_eq!(total, 1_000_000, "only the living division should contribute");
    }

    #[test]
    fn checked_scaled_demand_units_never_overflows_at_the_u64_boundary() {
        // usage_units・manpowerともにu64::MAXでも、u128へ拡張した積は
        // 数学的にu128::MAXを超えない(u64::MAX^2 < u128::MAX)。checked_mulが
        // 実際に失敗しないことを直接確認する(型境界の不変条件の実証)。
        let result = checked_scaled_demand_units(u64::MAX, u64::MAX, u64::MAX);
        assert!(result.is_ok(), "checked_mul must not overflow within u64 operand bounds");
    }

    #[test]
    fn checked_add_demand_reports_overflow_at_the_u128_boundary() {
        // 国家合計のchecked_addが実際にoverflowを検出できることを、u128::MAX境界で
        // 直接確認する(現実的なDivision数・manpowerでは到達しないが、演算そのものが
        // 安全であることをこの境界値で保証する)。
        assert_eq!(checked_add_demand(u128::MAX, 1), Err(SupplyDemandError::Overflow));
        assert_eq!(checked_add_demand(u128::MAX - 1, 1), Ok(u128::MAX));
    }

    // ─── カレンダー(spec §6.1・§12) ──────────────────────────────────

    #[test]
    fn new_game_start_day_has_thirty_events_before_production() {
        // 1800年1月1日開始、1月=31日。
        assert_eq!(events_before_next_monthly_production(1, 31).unwrap(), 30);
    }

    #[test]
    fn month_start_of_a_thirty_day_month() {
        assert_eq!(events_before_next_monthly_production(1, 30).unwrap(), 29);
    }

    #[test]
    fn mid_month() {
        assert_eq!(events_before_next_monthly_production(15, 31).unwrap(), 16);
    }

    #[test]
    fn last_day_of_month_has_zero_events_remaining() {
        assert_eq!(events_before_next_monthly_production(31, 31).unwrap(), 0);
    }

    #[test]
    fn one_day_before_month_end() {
        assert_eq!(events_before_next_monthly_production(30, 31).unwrap(), 1);
    }

    #[test]
    fn shortest_month_february() {
        assert_eq!(days_in_month(2).unwrap(), 28);
        assert_eq!(events_before_next_monthly_production(1, 28).unwrap(), 27);
    }

    #[test]
    fn longest_month_january() {
        assert_eq!(days_in_month(1).unwrap(), 31);
    }

    #[test]
    fn day_zero_is_an_explicit_calendar_error() {
        assert_eq!(
            events_before_next_monthly_production(0, 31),
            Err(CalendarError::DayOutOfRange { day: 0, days_in_month: 31 })
        );
    }

    #[test]
    fn day_beyond_days_in_month_is_an_explicit_calendar_error() {
        assert_eq!(
            events_before_next_monthly_production(32, 31),
            Err(CalendarError::DayOutOfRange { day: 32, days_in_month: 31 })
        );
    }

    #[test]
    fn month_zero_is_an_explicit_calendar_error() {
        assert_eq!(days_in_month(0), Err(CalendarError::InvalidMonth { month: 0 }));
    }

    #[test]
    fn month_thirteen_is_an_explicit_calendar_error() {
        assert_eq!(days_in_month(13), Err(CalendarError::InvalidMonth { month: 13 }));
    }

    // ─── ブリッジ必要量(spec §6・§14) ────────────────────────────────

    #[test]
    fn new_game_bridge_units_are_exactly_thirty_one_million() {
        // daily_demand_units=1_000_000(満編制Infantry1個師団)、events=30。
        let required = compute_required_bridge_units(1_000_000, 30).unwrap();
        assert_eq!(required, 31_000_000, "31.0 in canonical units");
        assert_eq!(military_supplies_units_to_value(required as u64), 31.0);
    }

    #[test]
    fn zero_demand_yields_zero_required_bridge() {
        assert_eq!(compute_required_bridge_units(0, 30).unwrap(), 0);
    }

    #[test]
    fn last_day_of_month_yields_one_day_of_pure_reserve() {
        let events = events_before_next_monthly_production(31, 31).unwrap();
        let required = compute_required_bridge_units(1_000_000, events).unwrap();
        assert_eq!(required, 1_000_000, "only the boundary-day reserve remains");
    }

    // ─── 1国家分の原子的付与(spec §8〜10) ────────────────────────────

    fn definitions_and_full_infantry(country: usize, div_id: usize) -> (
        HashMap<DivisionDefinitionId, DivisionDefinition>,
        HashMap<DivisionId, Division>,
    ) {
        (
            defs(vec![infantry_def(0)]),
            divs(vec![division(div_id, country, 0, 10_000, 10_000)]),
        )
    }

    #[test]
    fn new_game_scenario_grants_exactly_thirty_one_for_a_fresh_country() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::Granted { units: 31_000_000 });
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 31.0);
        assert!(country.military_supplies_initial_grant_applied);
    }

    #[test]
    fn already_granted_marker_prevents_any_recomputation() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.military_supplies_initial_grant_applied = true;
        // stockpileはあえて0のまま(spec §8.1: marker true・備蓄0でも再付与しない)。
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::AlreadyGranted);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.0);
    }

    #[test]
    fn calling_twice_in_sequence_only_grants_once() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        let after_first = country.stockpile.get(ResourceType::MilitarySupplies);
        let second = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(second, BridgeGrantOutcome::AlreadyGranted);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), after_first);
        assert_eq!(after_first, 31.0);
    }

    #[test]
    fn eliminated_country_gets_zero_grant_and_marker_true() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.status = CountryStatus::Eliminated {
            eliminated_date: "1800/06/01".to_string(),
            annexed_by: CountryId(1),
        };
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::EliminatedCountrySkipped);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.0);
        assert!(country.military_supplies_initial_grant_applied);
    }

    #[test]
    fn active_country_without_any_division_gets_zero_required_and_marker_true() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions: HashMap<DivisionId, Division> = HashMap::new();
        let mut country = test_country(0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::GrantedZero);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.0);
        assert!(country.military_supplies_initial_grant_applied);
    }

    #[test]
    fn old_save_top_up_required_31_current_0_grants_31() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 0.0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::Granted { units: 31_000_000 });
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 31.0);
    }

    #[test]
    fn old_save_top_up_required_31_current_10_grants_21() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 10.0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::Granted { units: 21_000_000 });
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 31.0);
    }

    #[test]
    fn old_save_top_up_required_31_current_31_grants_zero() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 31.0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::GrantedZero);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 31.0);
        assert!(country.military_supplies_initial_grant_applied);
    }

    #[test]
    fn old_save_top_up_required_31_current_50_does_not_decrease_stockpile() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 50.0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::GrantedZero);
        assert_eq!(
            country.stockpile.get(ResourceType::MilitarySupplies),
            50.0,
            "existing stockpile above the requirement must never be reduced"
        );
        assert!(country.military_supplies_initial_grant_applied);
    }

    #[test]
    fn marker_true_with_zero_stockpile_never_regrants() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.military_supplies_initial_grant_applied = true;
        country.stockpile.set(ResourceType::MilitarySupplies, 0.0);
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::AlreadyGranted);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.0);
    }

    #[test]
    fn mid_month_old_save_grants_the_remaining_days_worth() {
        // required = daily_demand(1.0) × (events(11) + 1) = 12.0、current=5.0 → grant=7.0。
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 5.0);
        // 1月20日(31日の月): events = 31 - 20 = 11、required = (11+1) = 12.0。
        let outcome = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            20,
            1,
        )
        .unwrap();
        assert_eq!(outcome, BridgeGrantOutcome::Granted { units: 7_000_000 });
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 12.0);
    }

    #[test]
    fn atomicity_invalid_current_stockpile_leaves_marker_false_and_stockpile_untouched() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country
            .stockpile
            .amounts
            .insert(ResourceType::MilitarySupplies, f64::NAN);
        let err = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap_err();
        assert_eq!(err, BridgeGrantError::InvalidCurrentStockpile);
        assert!(!country.military_supplies_initial_grant_applied);
        assert!(country.stockpile.get(ResourceType::MilitarySupplies).is_nan());
    }

    #[test]
    fn atomicity_invalid_division_leaves_marker_false_and_stockpile_untouched() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 11_000, 10_000)]); // corrupt: manpower>max
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::MilitarySupplies, 5.0);
        let err = apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap_err();
        assert!(matches!(err, BridgeGrantError::Demand(_)));
        assert!(!country.military_supplies_initial_grant_applied);
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 5.0);
    }

    #[test]
    fn one_countrys_failure_does_not_disturb_another_countrys_migration() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 11_000, 10_000), // 国家0: 不正(manpower>max)
            division(1, 1, 0, 10_000, 10_000), // 国家1: 正常
        ]);
        let mut countries = vec![test_country(0), test_country(1)];
        apply_military_supplies_bridge_migration(&mut countries, &divisions, &definitions, 1, 1);

        assert!(!countries[0].military_supplies_initial_grant_applied);
        assert_eq!(countries[0].stockpile.get(ResourceType::MilitarySupplies), 0.0);

        assert!(countries[1].military_supplies_initial_grant_applied);
        assert_eq!(countries[1].stockpile.get(ResourceType::MilitarySupplies), 31.0);
    }

    #[test]
    fn migration_over_multiple_countries_is_deterministic_regardless_of_slice_order() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 1, 0, 10_000, 10_000),
            division(2, 2, 0, 10_000, 10_000),
        ]);
        let mut order_a = vec![test_country(0), test_country(1), test_country(2)];
        let mut order_b = vec![test_country(2), test_country(0), test_country(1)];

        apply_military_supplies_bridge_migration(&mut order_a, &divisions, &definitions, 1, 1);
        apply_military_supplies_bridge_migration(&mut order_b, &divisions, &definitions, 1, 1);

        for id in [0usize, 1, 2] {
            let a = order_a.iter().find(|c| c.id == CountryId(id)).unwrap();
            let b = order_b.iter().find(|c| c.id == CountryId(id)).unwrap();
            assert_eq!(
                a.stockpile.get(ResourceType::MilitarySupplies),
                b.stockpile.get(ResourceType::MilitarySupplies)
            );
            assert_eq!(
                a.military_supplies_initial_grant_applied,
                b.military_supplies_initial_grant_applied
            );
        }
    }

    #[test]
    fn migration_applying_twice_stays_idempotent() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![division(0, 0, 0, 10_000, 10_000)]);
        let mut countries = vec![test_country(0)];
        apply_military_supplies_bridge_migration(&mut countries, &divisions, &definitions, 1, 1);
        let after_first = countries[0].stockpile.get(ResourceType::MilitarySupplies);
        apply_military_supplies_bridge_migration(&mut countries, &divisions, &definitions, 1, 1);
        assert_eq!(
            countries[0].stockpile.get(ResourceType::MilitarySupplies),
            after_first
        );
    }

    #[test]
    fn world_conservation_matches_sum_of_accepted_grants() {
        let definitions = defs(vec![infantry_def(0)]);
        let divisions = divs(vec![
            division(0, 0, 0, 10_000, 10_000),
            division(1, 1, 0, 5_000, 10_000),
        ]);
        let mut countries = vec![test_country(0), test_country(1)];
        let before: f64 = countries
            .iter()
            .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
            .sum();
        apply_military_supplies_bridge_migration(&mut countries, &divisions, &definitions, 1, 1);
        let after: f64 = countries
            .iter()
            .map(|c| c.stockpile.get(ResourceType::MilitarySupplies))
            .sum();
        assert_eq!(before, 0.0);
        assert_eq!(after, 31.0 + 15.5);
    }

    #[test]
    fn grant_uses_a_dedicated_api_and_never_touches_other_resources() {
        let (definitions, divisions) = definitions_and_full_infantry(0, 0);
        let mut country = test_country(0);
        country.stockpile.set(ResourceType::Food, 123.0);
        country.stockpile.set(ResourceType::MilitaryEquipment, 7.0);
        apply_military_supplies_bridge_grant_for_country(
            &mut country,
            &divisions,
            &definitions,
            1,
            1,
        )
        .unwrap();
        assert_eq!(country.stockpile.get(ResourceType::Food), 123.0);
        assert_eq!(country.stockpile.get(ResourceType::MilitaryEquipment), 7.0);
    }

    #[test]
    fn stockpile_is_a_default_country_stockpile_before_any_grant() {
        // 消滅国家判定・0付与のいずれもCountryStockpile::default()から安全に動作する。
        let country = CountryData {
            id: CountryId(0),
            stockpile: CountryStockpile::default(),
            ..CountryData::default()
        };
        assert_eq!(country.stockpile.get(ResourceType::MilitarySupplies), 0.0);
    }
}
