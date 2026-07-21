use crate::country::CountryData;
use crate::diplomacy::data::{DiplomaticRelation, TreatyType};
use crate::state::data::StateRegistry;

/// 提案スコア判定の決定論的内訳
#[derive(Debug, Clone)]
pub struct ProposalScoreItem {
    pub label: String,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct ProposalScoreBreakdown {
    pub items: Vec<ProposalScoreItem>,
    pub total_score: f32,
    pub required_score: f32,
    pub accepted: bool,
}

/// 外交提案の決定論的判定スコア計算関数
pub fn calculate_proposal_score(
    proposal: TreatyType,
    proposer: &CountryData,
    target: &CountryData,
    relation: &DiplomaticRelation,
    state_registry: &StateRegistry,
) -> ProposalScoreBreakdown {
    let mut items = Vec::new();

    // 1. 現在の関係値
    let op_score = relation.opinion * 0.5;
    items.push(ProposalScoreItem {
        label: "Current Opinion".to_string(),
        score: op_score,
    });

    // 2. 政治的価値観の近さ
    let v_prop = &proposer.politics.values;
    let v_targ = &target.politics.values;
    let diff = (v_prop.science_magic - v_targ.science_magic).abs()
        + (v_prop.individual_state - v_targ.individual_state).abs()
        + (v_prop.secular_religious - v_targ.secular_religious).abs();
    // 差が0なら+15.0、最大差600なら-15.0
    let val_score = 15.0 - (diff * 0.05);
    items.push(ProposalScoreItem {
        label: "Values Similarity".to_string(),
        score: val_score,
    });

    // 3. 既存条約による信頼
    if proposal == TreatyType::Alliance && relation.has_treaty(TreatyType::NonAggressionPact) {
        items.push(ProposalScoreItem {
            label: "Existing Non-Aggression Pact".to_string(),
            score: 15.0,
        });
    }

    // 4. 簡易国力比率 (州人口)
    let pop_prop: u64 = state_registry
        .states
        .iter()
        .filter(|s| s.owner_country_id == proposer.id)
        .map(|s| s.population)
        .sum();
    let pop_targ: u64 = state_registry
        .states
        .iter()
        .filter(|s| s.owner_country_id == target.id)
        .map(|s| s.population)
        .sum();

    if pop_targ > 0 {
        let ratio = pop_prop as f32 / pop_targ as f32;
        let power_score = ((ratio - 1.0) * 10.0).clamp(-15.0, 15.0);
        items.push(ProposalScoreItem {
            label: "Power Ratio".to_string(),
            score: power_score,
        });
    }

    // 5. 提案種別の基礎難易度
    let base_difficulty = match proposal {
        TreatyType::NonAggressionPact => -10.0,
        TreatyType::Alliance => -30.0,
    };
    items.push(ProposalScoreItem {
        label: "Base Difficulty".to_string(),
        score: base_difficulty,
    });

    let total_score: f32 = items.iter().map(|i| i.score).sum();
    let required_score = 10.0;
    let accepted = total_score >= required_score;

    ProposalScoreBreakdown {
        items,
        total_score,
        required_score,
        accepted,
    }
}
