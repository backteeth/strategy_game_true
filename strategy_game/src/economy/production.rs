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

/// 1つの国家が所有する州の生産・消費をまとめて一括処理する
pub fn process_country_production(
    states: &mut [&mut StateData],
    stockpile: &mut CountryStockpile,
    building_registry: &BuildingRegistry,
    is_bankrupt: bool,
) -> (f64, f64) {
    let mut science_total = 0.0;
    let mut magic_total = 0.0;

    // Step 1: 各州の各建物での入力資源要求量を集計する
    let mut total_inputs_needed: HashMap<ResourceType, f64> = HashMap::new();
    for state in states.iter() {
        // 経済用労働力比率(P23-004: population::update::calculate_economic_workforce_ratio
        // へ集約。ここでのみ既存どおり[0.0, 1.0]へクランプする)。
        let wf_ratio =
            calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce())
                .clamp(0.0, 1.0);
        let log_ratio = state.logistics_ratio;

        for (&b_type, &level) in &state.buildings {
            if level == 0 {
                continue;
            }
            if let Some(def) = building_registry.get(b_type) {
                let temp_op = wf_ratio.min(log_ratio);
                for (&res, &base_in) in &def.input_resources {
                    let needed = base_in * (level as f64) * (temp_op as f64);
                    *total_inputs_needed.entry(res).or_insert(0.0) += needed;
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

    // Step 3: 各州・建物ごとの最終稼働率を決定し、消費および生産を行う
    //
    // P26-003-REV-002: MilitarySuppliesは、個別の生産エントリ(1州×1建物出力)ごとに
    // 正規unitsへ量子化してから整数のまま国家合計へchecked_addで集計する
    // (quantize-then-sum、方式A、`SuppliesUnitsAccumulation`参照)。全州処理完了後に
    // 1回だけ`add_military_supplies_produced`を呼ぶ点(spec §12「国家単位で原子的」)は
    // P26-003から維持する——1国家が複数州に複数のMilitaryFactoryを持つ場合でも、
    // 真に1回のcommitで確定させるため。
    let mut military_supplies_units = SuppliesUnitsAccumulation::Accumulating(0);

    for state in states.iter_mut() {
        // 経済用労働力比率(P23-004: population::update::calculate_economic_workforce_ratio
        // へ集約。ここでのみ既存どおり[0.0, 1.0]へクランプする)。
        let wf_ratio =
            calculate_economic_workforce_ratio(state.employed_workforce, state.total_workforce())
                .clamp(0.0, 1.0);
        let log_ratio = state.logistics_ratio;

        let mut op_ratios = HashMap::new();

        for (&b_type, &level) in &state.buildings {
            if level == 0 {
                op_ratios.insert(b_type, 0.0);
                continue;
            }
            let def = match building_registry.get(b_type) {
                Some(d) => d,
                None => continue,
            };

            // 入力資源充足率の最小値を取得
            let mut min_res_sat = 1.0f32;
            for res in def.input_resources.keys() {
                if let Some(&sat) = resource_satisfaction.get(res) {
                    min_res_sat = min_res_sat.min(sat);
                }
            }

            let final_op = calculate_operation_ratio(wf_ratio, log_ratio, min_res_sat, is_bankrupt);
            op_ratios.insert(b_type, final_op);

            // 消費実行
            for (&res, &base_in) in &def.input_resources {
                let actual_in = calculate_actual_input(base_in, level, final_op);
                stockpile.consume(res, actual_in);
            }

            // 生産実行 (Mineの場合は州の鉱床情報を利用)
            // P21-009: MagicCrystal鉱床は専用のCrystalMine(固定産出量・鉱床ゲート付き)の
            // 管轄とし、汎用Mineでは採掘しない(精製ステップを迂回する経路を作らないため)。
            if b_type == BuildingType::Mine {
                for deposit in &state.resource_deposits {
                    if deposit.discovered && deposit.resource_type != ResourceType::MagicCrystal {
                        let out_amount =
                            calculate_actual_output(deposit.base_output, level, final_op);
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

        state.building_operation_ratios = op_ratios;
    }

    // P26-003-REV-002: 全州の集計が終わった後、この国家についてただ1回だけ正規APIへ
    // commitする(spec §12「国家単位で原子的」、P26-003から維持)。整数units集計が
    // 失敗状態(`Failed`)のままだった場合、または合計から正規f64値への変換・往復検証
    // (`verified_military_supplies_value`)が失敗した場合は、備蓄を変更せず警告のみ
    // 記録する——生の`add`へのフォールバックはしない(`Result`を無視しない、明示的に
    // 処理する)。合計0の場合は、意図的に`CountryStockpile`を一切変更しない(spec §9
    // 「合計0の場合は備蓄を更新しない処理を推奨」)。
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
                        if let Err(err) = stockpile.add_military_supplies_produced(value) {
                            warn!(
                                "MilitarySupplies production commit failed for this country \
                                 this month (stockpile left unchanged, other resources \
                                 unaffected): {:?}",
                                err
                            );
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
            }
        }
    }

    (science_total, magic_total)
}
