use crate::building::data::{BuildingRegistry, BuildingType};
use crate::economy::resources::{CountryStockpile, ResourceType};
use crate::state::data::StateData;
use std::collections::HashMap;

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
        let wf_ratio = if state.total_workforce() > 0 {
            (state.employed_workforce as f32 / state.total_workforce() as f32).clamp(0.0, 1.0)
        } else {
            1.0
        };
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
    for state in states.iter_mut() {
        let wf_ratio = if state.total_workforce() > 0 {
            (state.employed_workforce as f32 / state.total_workforce() as f32).clamp(0.0, 1.0)
        } else {
            1.0
        };
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
                    stockpile.add(res, out_amount);
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

    (science_total, magic_total)
}
