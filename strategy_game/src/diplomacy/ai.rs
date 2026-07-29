use crate::country::CountryData;
use crate::diplomacy::crisis::DiplomaticCrisis;
use crate::diplomacy::data::DiplomaticRelation;
use crate::state::data::StateData;

/// 外交要求の受諾可能性を計算する関数 (-100.0 〜 100.0)
/// 正の値なら受諾、負の値なら拒否に傾く
pub fn calculate_demand_acceptance(
    crisis: &DiplomaticCrisis,
    target_country: &CountryData,
    initiator_country: &CountryData,
    target_states: &[&StateData],
    relation: &DiplomaticRelation,
    target_allies_power: f32,    // 対象国の同盟国の推定戦力
    initiator_allies_power: f32, // 要求国の同盟国の推定戦力
) -> f32 {
    let mut score = 0.0;

    // 基本値: 拒否寄り
    score -= 20.0;

    // 軍事力の比較 (簡易計算)
    // 実際の軍事力データはCountryDataのパラメータから推定するか、別途渡す必要がある
    let target_power = target_country.available_manpower as f32 + target_allies_power;
    let initiator_power = initiator_country.available_manpower as f32 + initiator_allies_power;

    if initiator_power > target_power * 1.5 {
        score += 30.0; // 相手が圧倒的に強い場合は受諾しやすい
    } else if target_power > initiator_power * 1.2 {
        score -= 40.0; // 自分の方が強いなら拒否する
    }

    // 関係値
    score += relation.opinion * 0.2; // 関係が良いと少し受諾しやすい
    score += relation.trust * 0.1; // 信頼が高いと少し受諾しやすい
    score -= relation.tension * 0.3; // 緊張が高いと拒否しやすい
    score -= relation.threat * 0.5; // 脅威が高いと警戒して拒否しやすい

    // 対象州の評価
    let mut requested_states_value = 0.0;
    for state in target_states {
        if state.id == target_country.capital_state_id {
            score -= 200.0; // 首都は絶対に渡さない
        }

        // 人口や資源による価値
        let value = (state.population as f32 / 100000.0) + state.buildings.len() as f32 * 5.0;
        requested_states_value += value;

        // 統合度が低いと手放しやすい
        if state.integration < 50.0 {
            score += 10.0;
        }
    }

    score -= requested_states_value * 2.0;

    // 要求州の数が多いほど拒否
    if target_states.len() > 1 {
        score -= (target_states.len() as f32 - 1.0) * 15.0;
    }

    // 対象国の状況
    if relation.at_war {
        score += 40.0; // 別で戦争中なら受諾しやすい
    }

    if target_country.treasury < 0.0 {
        score += 10.0; // 借金中
    }

    // 危機のエスカレーション段階
    score -= crisis.international_concern * 0.5;

    score.clamp(-100.0, 100.0)
}
