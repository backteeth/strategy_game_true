use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::resources::{
    CountryStockpile, ResourceType, military_supplies_units_to_value,
    military_supplies_value_to_units,
};
use crate::population::update::calculate_economic_workforce_ratio;
use crate::state::data::StateData;
use bevy::log::warn;
use std::collections::HashMap;

/// P26-003-REV-002: 国家単位のMilitarySupplies生産集計状態。
///
/// 個別の生産エントリ(1州×1建物のMilitarySupplies出力)ごとに
/// `military_supplies_value_to_units`で正規units化(quantize-then-sum、方式A)して
/// から`u128`のchecked_addで合算する。P26-003(REV-001で指摘された旧実装)の
/// 「国家合計の生f64を集計してから最後に1回だけ量子化する」(sum-then-quantize、
/// 方式B)は使わない——`MilitaryEquipment`の既存quantize-then-sum規律と揃える。
///
/// 一度でも失敗する(個別出力が量子化不能、または整数合計がoverflowする)と
/// `Failed`へ遷移し、以降どれだけ正常なエントリが続いても回復しない
/// (spec §8「失敗状態を維持」)。
enum SuppliesUnitsAccumulation {
    Accumulating(u128),
    Failed,
}

impl SuppliesUnitsAccumulation {
    fn accumulate(self, out_amount: f64) -> Self {
        let total = match self {
            SuppliesUnitsAccumulation::Failed => return SuppliesUnitsAccumulation::Failed,
            SuppliesUnitsAccumulation::Accumulating(total) => total,
        };
        match military_supplies_value_to_units(out_amount) {
            Ok(units) => match total.checked_add(units as u128) {
                Some(new_total) => SuppliesUnitsAccumulation::Accumulating(new_total),
                None => SuppliesUnitsAccumulation::Failed,
            },
            Err(_) => SuppliesUnitsAccumulation::Failed,
        }
    }
}

/// 国家内で集計した整数units合計(`u128`)を、`add_military_supplies_produced`
/// (f64引数)へ渡すための正規`f64`値へ変換する(spec §9)。`u64`域を超える場合、
/// または`units→value→units`の往復が元のunitsと一致しない場合は`None`を返す
/// (`military_supplies_units_to_value`自体は無検査関数のため、呼び出し側でここの
/// 往復検証を行う)。
fn verified_military_supplies_value(total_units: u128) -> Option<f64> {
    let units_u64 = u64::try_from(total_units).ok()?;
    let value = military_supplies_units_to_value(units_u64);
    match military_supplies_value_to_units(value) {
        Ok(round_trip_units) if round_trip_units == units_u64 => Some(value),
        _ => None,
    }
}

/// 建物稼働率を求める純粋関数 (0.0〜1.0)
pub fn calculate_operation_ratio(
    workforce_ratio: f32,
    logistics_ratio: f32,
    input_resource_ratio: f32,
    is_bankrupt: bool,
) -> f32 {
    let mut ratio = workforce_ratio
        .min(logistics_ratio)
        .min(input_resource_ratio);
    if is_bankrupt {
        ratio *= 0.8; // 国庫赤字ペナルティ
    }
    ratio.clamp(0.0, 1.0)
}

/// 実際の生産量を計算する純粋関数
pub fn calculate_actual_output(base_output: f64, level: u32, operation_ratio: f32) -> f64 {
    base_output * (level as f64) * (operation_ratio as f64)
}

/// 実際の消費量を計算する純粋関数
pub fn calculate_actual_input(base_input: f64, level: u32, operation_ratio: f32) -> f64 {
    base_input * (level as f64) * (operation_ratio as f64)
}

/// P28-010-IMPL-002: IG備蓄の保護月数。IndustrialGoods備蓄が、MilitaryFactoryの月間IG需要
/// (Step 1と同じ`base × level × t`の州合計)のこの月数分以上あるとき、Iron不足時の
/// MilitaryFactoryへのIron優先配分が**全面的に**適用される(P29-007: 重み α = 1)。
/// これを下回る間は、IG備蓄に比例して α が 0〜1 で下がり、優先配分と需要比例配分の間を
/// 線形に補間する([`military_iron_priority_weight`])。境界(ちょうど 3 か月分)は α = 1。
pub const MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS: f64 = 3.0;

/// P29-007: D4 の優先配分の重み α = clamp(S / (K × D), 0, 1)。
///
/// - S: 月初のIG備蓄、D: MilitaryFactoryの月間IG需要(Step 1と同じ`base × level × t`の州合計)、
///   K = [`MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS`]。
/// - α = 0: 需要比例配分(D0)と一致。α = 1: 従来の D4 優先配分と一致。
/// - D ≤ 0(MilitaryFactoryが IG を要求しない): 従来仕様(ゲート `S < K × 0` は偽 → 優先配分を
///   適用)を維持して α = 1。非有限の備蓄は 0 として扱う(ゼロ除算・不正値を作らない)。
pub fn military_iron_priority_weight(ig_stock: f64, ig_want_military: f64) -> f64 {
    let reserve = MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS * ig_want_military;
    if reserve.is_nan() || reserve <= 0.0 || !reserve.is_finite() {
        return 1.0;
    }
    let stock = if ig_stock.is_finite() { ig_stock.max(0.0) } else { 0.0 };
    (stock / reserve).clamp(0.0, 1.0)
}

/// P29-007: 充足率(= 予算 / 需要)の補間 `(1 − α) × P + α × M`。α = 0 で P、α = 1 で M と
/// ビット一致する。`P`・`M` は同じ在庫・需要から得た 2 方式の配分(`M` の他消費者は`t`込み)。
fn blend_iron_satisfaction(alpha: f64, proportional: f64, priority: f64) -> f32 {
    ((1.0 - alpha) * proportional + alpha * priority).clamp(0.0, 1.0) as f32
}

/// P28-010-IMPL-002(PRE-003のD4): Iron不足時のMilitaryFactory優先配分の入力充足率。
///
/// 優先配分(M)が定義される条件(満たす場合のみ`Some`、それ以外は`None`=現行の需要比例配分D0):
/// 1. MilitaryFactoryのIron需要が正
/// 2. Ironの需要合計(MilitaryFactory + 他の消費者)が月初備蓄を超える(Iron不足)
///
/// P29-007: 旧 3.(IG備蓄 ≥ 3か月分という二択のゲート)は、重み α
/// ([`military_iron_priority_weight`])による M と D0 の線形補間へ置き換えた。
///
/// 戻り値は`(MilitaryFactoryのIron充足率, 他の消費者のIron充足率)`。
/// MilitaryFactoryは全入力のうち最も厳しい`備蓄/需要`(`mf_min_input_ratio`、Iron自身を含む)
/// だけを確保し、使えない分は予約しない。他の消費者は残りを共通の充足率で按分する。
fn military_iron_priority_satisfaction(
    iron_stock: f64,
    iron_want_military: f64,
    iron_want_others: f64,
    mf_min_input_ratio: f64,
) -> Option<(f32, f32)> {
    if iron_want_military <= 0.0 {
        return None;
    }
    if iron_want_military + iron_want_others <= iron_stock {
        return None;
    }
    let mf_ratio = mf_min_input_ratio.clamp(0.0, 1.0);
    let reserved = iron_want_military * mf_ratio;
    let others_sat = if iron_want_others > 0.0 {
        ((iron_stock - reserved) / iron_want_others).clamp(0.0, 1.0)
    } else {
        1.0
    };
    Some((mf_ratio as f32, others_sat as f32))
}

/// 稼働率`planned_op`のまま全入力が現在備蓄で賄えるならそのまま返す(従来と同一の
/// 消費・産出を維持する)。賄えない場合は、最も厳しい入力に合わせて、その消費量が
/// 備蓄を超えない最大の稼働率(f32の切り捨て側)へ引き下げる。
fn limit_operation_ratio_to_stock(
    planned_op: f32,
    level: u32,
    inputs: &HashMap<ResourceType, f64>,
    stockpile: &CountryStockpile,
) -> Option<f32> {
    let affordable = |op: f32| {
        inputs
            .iter()
            .all(|(&res, &base_in)| calculate_actual_input(base_in, level, op) <= stockpile.get(res))
    };
    if affordable(planned_op) {
        return Some(planned_op);
    }
    let mut cap = planned_op as f64;
    for (&res, &base_in) in inputs {
        let per_op = base_in * level as f64;
        if per_op > 0.0 {
            cap = cap.min(stockpile.get(res) / per_op);
        }
    }
    if !cap.is_finite() || cap <= 0.0 {
        return Some(0.0);
    }
    let mut op = (cap as f32).min(planned_op);
    // f32への丸めで備蓄を超える場合のみ、1ulずつ下げる(有限回で必ず収束する)。
    while op > 0.0 && !affordable(op) {
        op = f32::from_bits(op.to_bits() - 1);
    }
    Some(op.max(0.0))
}

/// 全入力を原子的に引き落とす。全入力の備蓄を事前確認し、すべて賄える場合だけ
/// 引き落とす。途中で失敗した場合は、引き落とし済みの分を戻して`false`を返す
/// (入力の一部だけが消費された状態を残さない)。
fn consume_inputs_atomically(
    stockpile: &mut CountryStockpile,
    inputs: &HashMap<ResourceType, f64>,
    level: u32,
    op: f32,
) -> bool {
    let mut order: Vec<(ResourceType, f64)> = inputs
        .iter()
        .map(|(&res, &base_in)| (res, calculate_actual_input(base_in, level, op)))
        .collect();
    order.sort_by_key(|(res, _)| ResourceType::ALL.iter().position(|r| r == res));
    if order.iter().any(|&(res, amount)| stockpile.get(res) < amount) {
        return false;
    }
    let mut done: Vec<(ResourceType, f64)> = Vec::new();
    for &(res, amount) in &order {
        if stockpile.consume(res, amount) {
            done.push((res, amount));
        } else {
            for &(r, a) in &done {
                stockpile.add(r, a);
            }
            return false;
        }
    }
    true
}

/// 1つの国家の月次生産の結果(研究力と、MilitarySuppliesの実産出)。
///
/// P29-002: `military_supplies_produced`は、この月次生産で**実際に備蓄へ反映された**
/// MilitarySuppliesの総量(FIX1・D4・入力不足・entryごとの量子化を反映した値。国家備蓄の
/// 純増減ではない)。産出が0だった場合は`Some(0.0)`。集計・commitに失敗して備蓄へ反映
/// できなかった場合は`None`(0とは区別する)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountryProductionOutcome {
    pub science: f64,
    pub magic: f64,
    pub military_supplies_produced: Option<f64>,
    /// P29-009: MilitaryFactory の **Iron 未充足の要求量**(Iron/月)。労働力・物流・Iron 以外の入力
    /// (月初の充足率と、消費前の備蓄による引き下げ)で既に制約された稼働率の上限と、実際の稼働率の
    /// 差に、Iron の基本入力 × レベルを掛けた値。実消費だけでは見えない「供給が足りず消費も減った」
    /// 状態を識別する観測値で、生産会計・配分には使わない。
    pub iron_unmet_military: f64,
    /// P29-009: Factory の Iron 未充足の要求量(同じ定義。観測用で、建設判断の需要には使わない)。
    pub iron_unmet_other: f64,
    /// P29-009: Mine がこの月に備蓄へ加えた Iron の量(供給能力の観測)。
    pub iron_produced: f64,
}

/// 1つの国家が所有する州の生産・消費をまとめて一括処理する
pub fn process_country_production(
    states: &mut [&mut StateData],
    stockpile: &mut CountryStockpile,
    building_registry: &BuildingRegistry,
    is_bankrupt: bool,
) -> (f64, f64) {
    let outcome =
        process_country_production_with_outcome(states, stockpile, building_registry, is_bankrupt);
    (outcome.science, outcome.magic)
}

/// `process_country_production`と同一の処理を行い、MilitarySuppliesの実産出も返す
/// (P29-002、表示専用の実績。生産式・処理順・配分は`process_country_production`と同一)。
pub fn process_country_production_with_outcome(
    states: &mut [&mut StateData],
    stockpile: &mut CountryStockpile,
    building_registry: &BuildingRegistry,
    is_bankrupt: bool,
) -> CountryProductionOutcome {
    let mut science_total = 0.0;
    let mut magic_total = 0.0;

    // P26-DET-001-REV-001: `states`スライス自体の要素順は、呼び出し元
    // (`economy::mod::handle_monthly_economy`)が`state_registry.states`
    // (`assets/data/states.ron`のファイル記載順——国家ごとの物語的グルーピングで
    // "additional states"セクションが後方に離れて出現するため、StateId昇順では
    // ない)からフィルタして構築した`Vec`の順序をそのまま引き継いだ、実質的に
    // 恣意的な順序である。P26-DET-001が州内で修正したのと全く同じ原因
    // (`CountryStockpile::consume`が現在庫との厳格な全額判定・部分消費なしで
    // あり、生産と消費が同じ`ResourceType`を扱う)が、州をまたいでも成立する
    // ——State A(Factory)がState B(MilitaryFactory)より先に処理されれば
    // Bの消費はAの同月産出を利用できるが、逆順ならできない、という実質的な
    // 消費成否の分岐が生じる(実測で確認済み、
    // `tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`)。
    //
    // ここでは、`states`内の位置(index)を永続的な`StateId`(`state.id.0`)昇順で
    // 並べ替えた添字列`order`を1度だけ構築し、Step 1・Step 3の両方でこの順序を
    // 使う。Bevy Entity ID、メモリアドレス、HashMap内部順、生成順のいずれにも
    // 依存しない。
    let mut order: Vec<usize> = (0..states.len()).collect();
    order.sort_by_key(|&i| states[i].id.0);

    // Step 1: 各州の各建物での入力資源要求量を集計する
    let mut total_inputs_needed: HashMap<ResourceType, f64> = HashMap::new();
    // P28-010-IMPL-002: 建物種別 × 入力資源ごとの需要合計(Step 1と同じ式・同じ加算順)。
    let mut wanted_by_building: HashMap<(BuildingType, ResourceType), f64> = HashMap::new();
    for &i in &order {
        let state = &*states[i];
        // 経済用労働力比率(P23-004: population::update::calculate_economic_workforce_ratio
        // へ集約。ここでのみ既存どおり[0.0, 1.0]へクランプする)。
        let wf_ratio =
            calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce())
                .clamp(0.0, 1.0);
        let log_ratio = state.logistics_ratio;

        // P26-DET-001: `state.buildings`(`HashMap<BuildingType, u32>`)の反復順は
        // Rust既定のRandomState(プロセスごとにランダムなSipHashシード)に依存し、
        // プロセス起動ごとに変わりうる。ここではその生反復順を使わず、
        // `BuildingType::ALL`の宣言順(永続的・国家非依存の全順序、
        // `country_ai.rs::find_proactive_mine_state`が`ResourceType::ALL`で
        // 既に用いているのと同じ方針)で固定する。
        for b_type in BuildingType::ALL {
            let level = state.buildings.get(&b_type).copied().unwrap_or(0);
            if level == 0 {
                continue;
            }
            if let Some(def) = building_registry.get(b_type) {
                let temp_op = wf_ratio.min(log_ratio);
                for (&res, &base_in) in &def.input_resources {
                    let needed = base_in * (level as f64) * (temp_op as f64);
                    *total_inputs_needed.entry(res).or_insert(0.0) += needed;
                    *wanted_by_building.entry((b_type, res)).or_insert(0.0) += needed;
                }
            }
        }
    }

    // Step 2: 入力資源充足率を計算する
    let mut resource_satisfaction: HashMap<ResourceType, f32> = HashMap::new();
    for (&res, &needed) in &total_inputs_needed {
        if needed <= 0.0 {
            resource_satisfaction.insert(res, 1.0);
        } else {
            let avail = stockpile.get(res);
            let sat = (avail / needed).clamp(0.0, 1.0) as f32;
            resource_satisfaction.insert(res, sat);
        }
    }

    // P28-010-IMPL-002: Iron不足時のMilitaryFactory優先配分(D4)。条件を満たさなければ
    // `None`で、以降の稼働率は従来の資源別充足率(D0)のまま。
    let iron_priority = {
        let want = |b: BuildingType, r: ResourceType| {
            wanted_by_building.get(&(b, r)).copied().unwrap_or(0.0)
        };
        let iron_want_military = want(BuildingType::MilitaryFactory, ResourceType::Iron);
        // MilitaryFactoryの全入力のうち最も厳しい `備蓄 / 需要`(`ResourceType::ALL`順)。
        let mut mf_min_input_ratio = 1.0f64;
        for res in ResourceType::ALL {
            let w = want(BuildingType::MilitaryFactory, res);
            if w > 0.0 {
                mf_min_input_ratio = mf_min_input_ratio.min(stockpile.get(res) / w);
            }
        }
        // 他の消費者のIron需要(`BuildingType::ALL`順で加算)。
        let mut iron_want_others = 0.0f64;
        for b in BuildingType::ALL {
            if b != BuildingType::MilitaryFactory {
                iron_want_others += want(b, ResourceType::Iron);
            }
        }
        // P29-007: 優先配分(M)と需要比例配分(P)の補間の重み α(IG 備蓄 / 3 か月分)。
        let alpha = military_iron_priority_weight(
            stockpile.get(ResourceType::IndustrialGoods),
            want(BuildingType::MilitaryFactory, ResourceType::IndustrialGoods),
        );
        military_iron_priority_satisfaction(
            stockpile.get(ResourceType::Iron),
            iron_want_military,
            iron_want_others,
            mf_min_input_ratio,
        )
        .map(|(mf_sat, others_sat)| (mf_sat, others_sat, alpha))
    };

    // Step 3: 各州・建物ごとの最終稼働率を決定し、消費および生産を行う
    //
    // P26-003-REV-002: MilitarySuppliesは、個別の生産エントリ(1州×1建物出力)ごとに
    // 正規unitsへ量子化してから整数のまま国家合計へchecked_addで集計する
    // (quantize-then-sum、方式A、`SuppliesUnitsAccumulation`参照)。全州処理完了後に
    // 1回だけ`add_military_supplies_produced`を呼ぶ点(spec §12「国家単位で原子的」)は
    // P26-003から維持する——1国家が複数州に複数のMilitaryFactoryを持つ場合でも、
    // 真に1回のcommitで確定させるため。
    let mut military_supplies_units = SuppliesUnitsAccumulation::Accumulating(0);
    // P29-009: Iron の未充足の要求量・Mine 産出の観測(配分・会計には影響しない)。
    let mut iron_unmet_military = 0.0f64;
    let mut iron_unmet_other = 0.0f64;
    let mut iron_produced = 0.0f64;

    // P26-DET-001-REV-001: 各州ごとの`building_operation_ratios`は、下の全州分
    // 処理が終わってから1回だけ書き戻す(ループ構造をBuildingType外側/State内側へ
    // 反転させたため、州ごとに閉じたループの最後で書き戻すことができない)。
    let mut op_ratios_per_state: Vec<HashMap<BuildingType, f32>> =
        (0..states.len()).map(|_| HashMap::new()).collect();

    // P26-DET-001-REV-001: 根本原因はP26-DET-001と全く同じ形で州をまたいで再現する
    // (`tests/p26_det_001_rev_001_cross_state_production_order_e2e_test.rs`
    // Test A/Bで実測確認済み)。ある州の建物の生産(`add`)と、別の州の建物の消費
    // (`consume`、現在庫との厳格な全額判定・部分消費なし)が同じ`ResourceType`を
    // 扱う場合、州の処理順序(=呼び出し元が渡した`states`スライスの要素順、実体は
    // `assets/data/states.ron`のファイル記載順という恣意的な順序)によって消費の
    // 成否そのものが変わってしまう。
    //
    // ここでは、ループ構造を「State外側→BuildingType内側」から
    // 「BuildingType外側(`BuildingType::ALL`宣言順)→State内側(`StateId`昇順、
    // 上で構築した`order`)」へ反転させる。これにより、依存関係上先に処理される
    // べきBuildingType(例: Mine→Factory→MilitaryFactory)は、どの州にあるかに
    // 関わらず全州分先に処理されてから、後続のBuildingTypeが全州分処理される
    // ——生産依存順がStateIdより常に優先される。同じBuildingType内では
    // `order`(StateId昇順)がtie-breakとなる。この州が保有しない建物種別は
    // 元コード同様に一切触れない(avg_op計算[`economy/mod.rs`]が参照する
    // `building_operation_ratios`へ余計な0.0エントリを増やさない)。
    for b_type in BuildingType::ALL {
        for &i in &order {
            let state = &mut *states[i];

            // 経済用労働力比率(P23-004: population::update::calculate_economic_workforce_ratio
            // へ集約。ここでのみ既存どおり[0.0, 1.0]へクランプする)。
            let wf_ratio = calculate_economic_workforce_ratio(
                state.employed_workforce,
                state.total_workforce(),
            )
            .clamp(0.0, 1.0);
            let log_ratio = state.logistics_ratio;

            let Some(&level) = state.buildings.get(&b_type) else {
                continue;
            };
            if level == 0 {
                op_ratios_per_state[i].insert(b_type, 0.0);
                continue;
            }
            let def = match building_registry.get(b_type) {
                Some(d) => d,
                None => continue,
            };

            // 入力資源充足率の最小値を取得
            // (P28-010-IMPL-002: Iron不足でD4が成立する月は、Ironだけ建物種別ごとの値を使う。
            //  MilitaryFactoryは優先配分分、それ以外のIron消費者は残りの按分値)
            let mut min_res_sat = 1.0f32;
            for res in def.input_resources.keys() {
                let sat = match (*res, iron_priority) {
                    (ResourceType::Iron, Some((mf_sat, others_sat, alpha))) => {
                        // 優先配分の残り(予算)を、他の消費者の需要(`t`込み)に対する割合`others_sat`で
                        // 受け取る。消費は`base × level × op`(`t`なし)なので、`t × others_sat`で
                        // 受け取った予算ちょうどに揃える(`min(t, sat)`のままだと1/t倍に超過する)。
                        let priority = if b_type == BuildingType::MilitaryFactory {
                            mf_sat
                        } else {
                            others_sat * wf_ratio.min(log_ratio)
                        };
                        // P29-007: 需要比例配分(P: 全消費者に共通の月初備蓄/需要)との線形補間。
                        // 配分は建物ごとの Iron 量(`base × level × 充足率`)で、充足率の補間は
                        // その量の補間と等価。α = 1 で従来の D4、α = 0 で D0 とビット一致する。
                        let proportional =
                            resource_satisfaction.get(res).copied().unwrap_or(1.0);
                        Some(blend_iron_satisfaction(
                            alpha,
                            proportional as f64,
                            priority as f64,
                        ))
                    }
                    _ => resource_satisfaction.get(res).copied(),
                };
                if let Some(sat) = sat {
                    min_res_sat = min_res_sat.min(sat);
                }
            }

            let planned_op =
                calculate_operation_ratio(wf_ratio, log_ratio, min_res_sat, is_bankrupt);

            // P29-009: Iron を除いた入力で到達できる稼働率の上限(Iron 以外の入力の月初充足率と、
            // 消費前の備蓄による引き下げ)。実際の稼働率との差が「Iron による未充足」。観測だけで、
            // 以降の配分・消費には使わない。
            let iron_free_op = if matches!(
                b_type,
                BuildingType::MilitaryFactory | BuildingType::Factory
            ) && def.input_resources.contains_key(&ResourceType::Iron)
            {
                let mut other_sat = 1.0f32;
                let mut other_inputs: HashMap<ResourceType, f64> = HashMap::new();
                for (&r, &b) in &def.input_resources {
                    if r != ResourceType::Iron {
                        other_inputs.insert(r, b);
                        if let Some(&s) = resource_satisfaction.get(&r) {
                            other_sat = other_sat.min(s);
                        }
                    }
                }
                let cap = calculate_operation_ratio(wf_ratio, log_ratio, other_sat, is_bankrupt);
                Some(
                    limit_operation_ratio_to_stock(cap, level, &other_inputs, stockpile)
                        .unwrap_or(0.0),
                )
            } else {
                None
            };

            // P28-010-IMPL-001: 原子的な入力会計。`sat`は月初備蓄を、`t`込みの需要で
            // 割った値のため、`sat < t`の月は実消費が備蓄を超え得る。この時点の備蓄で
            // 全入力を賄える最大稼働率へ引き下げ、入力が引き落とせた分だけ産出する。
            let final_op = match limit_operation_ratio_to_stock(
                planned_op,
                level,
                &def.input_resources,
                stockpile,
            ) {
                Some(op) if consume_inputs_atomically(stockpile, &def.input_resources, level, op) => {
                    op
                }
                _ => 0.0,
            };
            op_ratios_per_state[i].insert(b_type, final_op);
            if let Some(free_op) = iron_free_op {
                let base_iron = def
                    .input_resources
                    .get(&ResourceType::Iron)
                    .copied()
                    .unwrap_or(0.0);
                let unmet = base_iron * level as f64 * (free_op - final_op).max(0.0) as f64;
                if b_type == BuildingType::MilitaryFactory {
                    iron_unmet_military += unmet;
                } else {
                    iron_unmet_other += unmet;
                }
            }
            if final_op <= 0.0 && planned_op > 0.0 {
                // 入力を1つも引き落とせなかったentryは産出・研究力を加算しない。
                continue;
            }

            // 生産実行 (Mineの場合は州の鉱床情報を利用)
            // P21-009: MagicCrystal鉱床は専用のCrystalMine(固定産出量・鉱床ゲート付き)の
            // 管轄とし、汎用Mineでは採掘しない(精製ステップを迂回する経路を作らないため)。
            if b_type == BuildingType::Mine {
                for deposit in &state.resource_deposits {
                    if deposit.discovered && deposit.resource_type != ResourceType::MagicCrystal {
                        let out_amount =
                            calculate_actual_output(deposit.base_output, level, final_op);
                        if deposit.resource_type == ResourceType::Iron {
                            iron_produced += out_amount;
                        }
                        stockpile.add(deposit.resource_type, out_amount);
                    }
                }
            } else {
                for (&res, &base_out) in &def.output_resources {
                    let out_amount = calculate_actual_output(base_out, level, final_op);
                    if res == ResourceType::MilitaryEquipment {
                        // P25-002-REV-001: MilitaryEquipmentだけは固定小数点units経由で
                        // 加算する(生の`f64`加算は2^53付近で精度が崩れるため)。他の
                        // ResourceTypeの生産経路(このelse節を含む)は無変更。失敗時
                        // (値域超過等、現実的な到達可能量では発生しない)は備蓄を
                        // 変更しない——生の`add`へフォールバックしない。
                        let _ = stockpile.add_military_equipment_produced(out_amount);
                    } else if res == ResourceType::MilitarySupplies {
                        // P26-003-REV-002: この個別エントリ(1州×1建物出力)を直ちに
                        // 正規unitsへ量子化し、整数のまま国家合計へchecked_addで集計する
                        // (Step 3コメント参照)。生f64の合算(旧P26-003の
                        // `military_supplies_total_value += out_amount`)は行わない。
                        // ここでは`CountryStockpile`を一切変更しない(即時commitはせず、
                        // 全州処理完了後に1回だけcommitする)。
                        military_supplies_units = military_supplies_units.accumulate(out_amount);
                    } else {
                        stockpile.add(res, out_amount);
                    }
                }
            }

            // 研究力出力
            if def.science_output > 0.0 {
                science_total += calculate_actual_output(def.science_output, level, final_op);
            }
            if def.magic_output > 0.0 {
                magic_total += calculate_actual_output(def.magic_output, level, final_op);
            }
        }
    }

    for (i, state) in states.iter_mut().enumerate() {
        state.building_operation_ratios = std::mem::take(&mut op_ratios_per_state[i]);
    }

    // P26-003-REV-002: 全州の集計が終わった後、この国家についてただ1回だけ正規APIへ
    // commitする(spec §12「国家単位で原子的」、P26-003から維持)。整数units集計が
    // 失敗状態(`Failed`)のままだった場合、または合計から正規f64値への変換・往復検証
    // (`verified_military_supplies_value`)が失敗した場合は、備蓄を変更せず警告のみ
    // 記録する——生の`add`へのフォールバックはしない(`Result`を無視しない、明示的に
    // 処理する)。合計0の場合は、意図的に`CountryStockpile`を一切変更しない(spec §9
    // 「合計0の場合は備蓄を更新しない処理を推奨」)。
    // P29-002: 実際に備蓄へ反映された量(表示専用)。commitに成功した場合だけ`Some`。
    let mut military_supplies_produced: Option<f64> = None;
    match military_supplies_units {
        SuppliesUnitsAccumulation::Failed => {
            warn!(
                "MilitarySupplies production commit failed for this country this month \
                 (an individual production entry could not be quantized to units, or the \
                 running integer total overflowed; stockpile left unchanged, other resources \
                 unaffected)"
            );
        }
        SuppliesUnitsAccumulation::Accumulating(total_units) => {
            if total_units > 0 {
                match verified_military_supplies_value(total_units) {
                    Some(value) => {
                        match stockpile.add_military_supplies_produced(value) {
                            Ok(()) => military_supplies_produced = Some(value),
                            Err(err) => {
                                warn!(
                                    "MilitarySupplies production commit failed for this country \
                                     this month (stockpile left unchanged, other resources \
                                     unaffected): {:?}",
                                    err
                                );
                            }
                        }
                    }
                    None => {
                        warn!(
                            "MilitarySupplies production commit failed for this country this \
                             month (accumulated integer units total could not be converted \
                             back to a valid value; stockpile left unchanged, other resources \
                             unaffected)"
                        );
                    }
                }
            } else {
                military_supplies_produced = Some(0.0);
            }
        }
    }

    CountryProductionOutcome {
        science: science_total,
        magic: magic_total,
        military_supplies_produced,
        iron_unmet_military,
        iron_unmet_other,
        iron_produced,
    }
}

#[cfg(test)]
mod atomic_input_tests {
    use super::*;

    fn inputs() -> HashMap<ResourceType, f64> {
        HashMap::from([(ResourceType::Iron, 30.0), (ResourceType::IndustrialGoods, 15.0)])
    }

    #[test]
    fn atomic_consume_leaves_stock_unchanged_when_one_input_is_short() {
        let mut s = CountryStockpile::new();
        s.set(ResourceType::Iron, 100.0);
        s.set(ResourceType::IndustrialGoods, 1.0);
        assert!(!consume_inputs_atomically(&mut s, &inputs(), 1, 1.0));
        assert_eq!(s.get(ResourceType::Iron), 100.0, "partial deduction must not remain");
        assert_eq!(s.get(ResourceType::IndustrialGoods), 1.0);
    }

    #[test]
    fn atomic_consume_deducts_every_input_when_all_are_available() {
        let mut s = CountryStockpile::new();
        s.set(ResourceType::Iron, 100.0);
        s.set(ResourceType::IndustrialGoods, 100.0);
        assert!(consume_inputs_atomically(&mut s, &inputs(), 2, 0.5));
        assert_eq!(s.get(ResourceType::Iron), 70.0);
        assert_eq!(s.get(ResourceType::IndustrialGoods), 85.0);
    }

    #[test]
    fn limit_keeps_planned_op_when_affordable_and_reduces_otherwise() {
        let mut s = CountryStockpile::new();
        s.set(ResourceType::Iron, 30.0);
        s.set(ResourceType::IndustrialGoods, 15.0);
        assert_eq!(limit_operation_ratio_to_stock(1.0, 1, &inputs(), &s), Some(1.0));
        s.set(ResourceType::IndustrialGoods, 7.5);
        let op = limit_operation_ratio_to_stock(1.0, 1, &inputs(), &s).unwrap();
        assert!(op <= 0.5 && op > 0.499_999);
        s.set(ResourceType::Iron, 0.0);
        assert_eq!(limit_operation_ratio_to_stock(1.0, 1, &inputs(), &s), Some(0.0));
    }
}

#[cfg(test)]
mod military_iron_priority_tests {
    use super::*;

    // iron_stock, iron_want_military, iron_want_others, mf_min_input_ratio
    #[test]
    fn iron_sufficient_keeps_the_proportional_rule() {
        assert_eq!(military_iron_priority_satisfaction(100.0, 30.0, 40.0, 1.0), None);
        // 需要合計 = 備蓄(ちょうど)も「不足ではない」。
        assert_eq!(military_iron_priority_satisfaction(70.0, 30.0, 40.0, 1.0), None);
    }

    #[test]
    fn no_military_factory_demand_keeps_the_proportional_rule() {
        assert_eq!(military_iron_priority_satisfaction(10.0, 0.0, 40.0, 1.0), None);
    }

    /// P29-007: 重み α = clamp(S / (3 × D), 0, 1)。端点・中間・境界・不正値。
    #[test]
    fn weight_is_a_clamped_linear_ramp_of_the_ig_stock() {
        let d = 15.0;
        let boundary = MILITARY_IRON_PRIORITY_IG_RESERVE_MONTHS * d;
        assert_eq!(military_iron_priority_weight(0.0, d), 0.0, "IG 備蓄 0 は需要比例配分");
        assert_eq!(military_iron_priority_weight(boundary, d), 1.0, "ちょうど 3 か月分は優先配分");
        assert_eq!(military_iron_priority_weight(boundary * 4.0, d), 1.0, "3 か月分以上は優先配分");
        assert!((military_iron_priority_weight(boundary / 2.0, d) - 0.5).abs() < 1e-12);
        assert!((military_iron_priority_weight(boundary / 4.0, d) - 0.25).abs() < 1e-12);
        // 直前・直後に大きな飛躍がない(旧仕様は 0 → 1 の二択)。
        let below = military_iron_priority_weight(f64::from_bits(boundary.to_bits() - 1), d);
        assert!(below > 1.0 - 1e-12 && below <= 1.0);
        // 単調非減少。
        let mut prev = 0.0;
        for i in 0..=200 {
            let a = military_iron_priority_weight(boundary * i as f64 / 100.0, d);
            assert!(a >= prev && (0.0..=1.0).contains(&a));
            prev = a;
        }
    }

    #[test]
    fn weight_is_well_defined_for_zero_demand_and_invalid_values() {
        // D = 0: 従来仕様(ゲート `S < K × 0` は偽 → 優先配分を適用)を維持して α = 1。
        assert_eq!(military_iron_priority_weight(0.0, 0.0), 1.0);
        assert_eq!(military_iron_priority_weight(100.0, 0.0), 1.0);
        assert_eq!(military_iron_priority_weight(5.0, -1.0), 1.0);
        // 非有限・負の備蓄は 0 として扱う(NaN を作らない)。
        assert_eq!(military_iron_priority_weight(f64::NAN, 15.0), 0.0);
        assert_eq!(military_iron_priority_weight(-3.0, 15.0), 0.0);
        assert_eq!(military_iron_priority_weight(f64::INFINITY, 15.0), 0.0);
        assert_eq!(military_iron_priority_weight(10.0, f64::NAN), 1.0);
    }

    #[test]
    fn blend_matches_the_endpoints_bit_for_bit_and_interpolates_between() {
        let (p, m) = (0.3_f32 as f64, 0.9_f32 as f64);
        assert_eq!(blend_iron_satisfaction(0.0, p, m), 0.3_f32);
        assert_eq!(blend_iron_satisfaction(1.0, p, m), 0.9_f32);
        let mid = blend_iron_satisfaction(0.5, p, m);
        assert!((mid - 0.6).abs() < 1e-6);
        // 補間値は 2 端点の間に収まる。
        for i in 0..=100 {
            let a = i as f64 / 100.0;
            let v = blend_iron_satisfaction(a, p, m) as f64;
            assert!(v >= p - 1e-6 && v <= m + 1e-6);
        }
    }

    #[test]
    fn military_gets_its_demand_first_and_others_share_the_rest() {
        let (mf, others) = military_iron_priority_satisfaction(50.0, 30.0, 40.0, 1.0).unwrap();
        assert_eq!(mf, 1.0);
        assert!((others - 0.5).abs() < 1e-6, "others={others}"); // (50-30)/40
    }

    #[test]
    fn military_is_capped_by_stock_and_others_get_nothing() {
        let (mf, others) =
            military_iron_priority_satisfaction(20.0, 30.0, 40.0, 20.0 / 30.0).unwrap();
        assert!((mf - 20.0 / 30.0).abs() < 1e-6);
        assert_eq!(others, 0.0);
    }

    #[test]
    fn iron_is_not_reserved_beyond_what_another_military_input_allows() {
        // 他の入力が律速(0.4)なら、MilitaryFactoryのIron予約は 30 × 0.4 = 12 だけ。
        let (mf, others) = military_iron_priority_satisfaction(50.0, 30.0, 40.0, 0.4).unwrap();
        assert!((mf - 0.4).abs() < 1e-6);
        assert!((others - (50.0 - 12.0) / 40.0).abs() < 1e-6, "others={others}");
    }

    #[test]
    fn zero_stock_and_zero_others_are_well_defined() {
        let (mf, others) = military_iron_priority_satisfaction(0.0, 30.0, 40.0, 0.0).unwrap();
        assert_eq!((mf, others), (0.0, 0.0));
        let (mf, others) =
            military_iron_priority_satisfaction(10.0, 30.0, 0.0, 10.0 / 30.0).unwrap();
        assert!((mf - 10.0 / 30.0).abs() < 1e-6);
        assert_eq!(others, 1.0);
    }
}
