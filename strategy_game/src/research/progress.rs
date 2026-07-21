use crate::research::allocation::ResearchAllocation;
use crate::research::data::TechnologyField;

/// 研究力・分野別効率・月次研究ポイント計算関数
pub fn calculate_field_monthly_points(
    science_cap: f64,
    magic_cap: f64,
    base_cap: f64,
    allocation: &ResearchAllocation,
    military_bonus: f64,
    fusion_tech_mult: f64,
) -> (f64, [f64; 4]) {
    let total_cap = science_cap + magic_cap + base_cap;
    let spec_sum = (science_cap + magic_cap).max(0.001);

    let science_share = science_cap / spec_sum;
    let magic_share = magic_cap / spec_sum;

    let science_eff = 0.75 + (science_share * 0.5);
    let magic_eff = 0.75 + (magic_share * 0.5);
    let military_eff = 1.0 + military_bonus;

    let fusion_balance = 2.0 * science_cap.min(magic_cap) / spec_sum;
    let fusion_eff = fusion_balance * fusion_tech_mult;

    let sci_pts = total_cap * (allocation.science as f64) * science_eff;
    let mag_pts = total_cap * (allocation.magic as f64) * magic_eff;
    let mil_pts = total_cap * (allocation.military as f64) * military_eff;
    let fus_pts = total_cap * (allocation.fusion as f64) * fusion_eff;

    let points_arr = [sci_pts, mag_pts, mil_pts, fus_pts];
    (total_cap, points_arr)
}

pub fn get_field_points(points_arr: &[f64; 4], field: TechnologyField) -> f64 {
    match field {
        TechnologyField::Science => points_arr[0],
        TechnologyField::Magic => points_arr[1],
        TechnologyField::Military => points_arr[2],
        TechnologyField::Fusion => points_arr[3],
    }
}
