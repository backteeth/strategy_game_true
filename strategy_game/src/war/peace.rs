use crate::app::time::GameDate;
use crate::common::{CountryId, StateId, WarId};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::{WarRegistry, WarStatus};
use serde::{Deserialize, Serialize};

/// 講和条件の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeaceTerm {
    /// 白紙講和
    WhitePeace,
    /// 戦争目標地域の割譲
    CedeWarGoalRegion,
    /// 攻撃側の敗北受諾
    AttackerConcedes,
}

impl PeaceTerm {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    /// `War.applied_terms`にもこのキーがそのまま(未翻訳で)格納され、表示直前に解決される。
    pub fn display_name(self) -> &'static str {
        match self {
            PeaceTerm::WhitePeace => "peace_term.white_peace",
            PeaceTerm::CedeWarGoalRegion => "peace_term.cede_war_goal_region",
            PeaceTerm::AttackerConcedes => "peace_term.attacker_concedes",
        }
    }
}

/// 講和提案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeaceOffer {
    pub war_id: WarId,
    pub proposer_country_id: CountryId,
    pub recipient_country_id: CountryId,
    pub term: PeaceTerm,
    pub created_date: String,
}

/// AIまたはゲームロジックによる講和提案の受諾判定
pub fn can_accept_peace_offer(
    offer: &PeaceOffer,
    war_registry: &WarRegistry,
    state_registry: Option<&StateRegistry>,
    military_registry: Option<&MilitaryRegistry>,
    current_date_str: &str,
) -> Result<(), &'static str> {
    // P20-009: 戻り値は表示用の英語原文ではなく安定した翻訳キー(war_error.peace.*)。
    // 呼び出し元UI(peace_panel)が`localization::t()`で表示直前に言語へ解決する。
    let war = war_registry
        .wars
        .get(&offer.war_id)
        .ok_or("war_error.peace.war_not_found")?;

    if war.status != WarStatus::Active {
        return Err("war_error.peace.already_ended");
    }

    // 提案者が戦争参加国か検証
    let is_attacker = war.attackers.contains(&offer.proposer_country_id);
    let is_defender = war.defenders.contains(&offer.proposer_country_id);

    if !is_attacker && !is_defender {
        return Err("war_error.peace.not_participant");
    }

    let is_recipient_valid = (is_attacker && war.defenders.contains(&offer.recipient_country_id))
        || (is_defender && war.attackers.contains(&offer.recipient_country_id));
    if !is_recipient_valid {
        return Err("war_error.peace.invalid_recipient");
    }

    match offer.term {
        PeaceTerm::WhitePeace => {
            // 条件1: 戦争開始から30日以上経過
            let start = GameDate::from_string(&war.start_date).unwrap_or_default();
            let current = GameDate::from_string(current_date_str).unwrap_or_default();
            let elapsed_days = current.days_since(&start);

            if elapsed_days < 30 {
                return Err("war_error.peace.white_peace_too_early");
            }

            // 条件2: 攻撃側戦勝点が -10 以上 (防御側視点で有利すぎない)
            if war.war_score < -10.0 {
                return Err("war_error.peace.white_peace_score_too_low");
            }
        }
        PeaceTerm::CedeWarGoalRegion => {
            // 攻撃側のみが割譲要求を行える
            if !is_attacker {
                return Err("war_error.peace.cede_not_attacker");
            }

            // 戦争目標地域の法的所有国が防御側か検証
            let target_state_id = war
                .war_goals
                .first()
                .and_then(|g| g.target_states.first().copied())
                .ok_or("war_error.peace.no_target_state")?;

            if let Some(s_reg) = state_registry {
                let state = s_reg
                    .get(target_state_id)
                    .ok_or("war_error.peace.target_state_missing")?;
                if state.owner_country_id != offer.recipient_country_id {
                    return Err("war_error.peace.target_not_owned_by_defender");
                }
            }

            // 受諾判定: 戦勝点60以上 または 防御側が降伏条件を満たしている
            let defender_capitulated = match (state_registry, military_registry) {
                (Some(s_reg), Some(m_reg)) => {
                    crate::war::capitulation::check_defender_capitulation(war, s_reg, m_reg)
                }
                _ => false,
            };

            if war.war_score < 60.0 && !defender_capitulated {
                return Err("war_error.peace.cede_insufficient");
            }
        }
        PeaceTerm::AttackerConcedes => {
            // 攻撃側のみが敗北受諾を提案できる
            if !is_attacker {
                return Err("war_error.peace.concede_not_attacker");
            }
        }
    }

    Ok(())
}

/// 戦争終結後の敵国領残存陸軍の自国領域への安全帰還処理
pub fn relocate_hostile_territory_divisions(
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
) {
    for division in military_registry.divisions.values_mut() {
        if division.status == DivisionStatus::Destroyed || division.manpower == 0 {
            continue;
        }

        let curr_state = match state_registry.get(division.current_state) {
            Some(s) => s,
            None => continue,
        };

        // 自国所有または自国支配の地域に居る場合は移動不要
        if curr_state.owner_country_id == division.owner || curr_state.controller() == division.owner {
            continue;
        }

        // 帰還優先1: 隣接する自国支配/所有の陸上地域を地域ID順に検索
        let mut valid_neighbors: Vec<StateId> = curr_state
            .neighbors
            .iter()
            .copied()
            .filter(|&nid| {
                if let Some(ns) = state_registry.get(nid) {
                    !ns.is_sea
                        && (ns.controller() == division.owner || ns.owner_country_id == division.owner)
                } else {
                    false
                }
            })
            .collect();

        valid_neighbors.sort_by_key(|s| s.0);

        if let Some(&target_neighbor) = valid_neighbors.first() {
            division.current_state = target_neighbor;
            division.status = DivisionStatus::Idle;
            division.target_state = None;
            division.current_path.clear();
            division.movement_progress = 0.0;
            continue;
        }

        // 帰還優先2: 自国支配/所有の全陸上地域の中で最小地域IDへ配置
        let mut owned_states: Vec<StateId> = state_registry
            .states
            .iter()
            .filter(|s| {
                !s.is_sea && (s.controller() == division.owner || s.owner_country_id == division.owner)
            })
            .map(|s| s.id)
            .collect();

        owned_states.sort_by_key(|s| s.0);

        if let Some(&min_state_id) = owned_states.first() {
            division.current_state = min_state_id;
            division.status = DivisionStatus::Idle;
            division.target_state = None;
            division.current_path.clear();
            division.movement_progress = 0.0;
            continue;
        }

        // 帰還優先3: 自国領土が一切ない場合は安全に削除/非活動化
        division.status = DivisionStatus::Destroyed;
        division.manpower = 0;
        division.organization = 0.0;
        division.target_state = None;
        division.current_path.clear();
    }
}

/// 講和条約の終結処理を実行（二重適用防止、領土移転、戦闘中止、占領リセット、休戦作成、前線削除）
#[allow(clippy::too_many_arguments)]
pub fn execute_peace_settlement(
    war_id: WarId,
    term: PeaceTerm,
    end_reason: &str,
    current_date_str: &str,
    state_registry: &mut StateRegistry,
    war_registry: &mut WarRegistry,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
) -> Result<(), &'static str> {
    let war = war_registry
        .wars
        .get_mut(&war_id)
        .ok_or("war_error.peace.war_not_found")?;

    // 二重適用防止
    if war.status != WarStatus::Active {
        return Ok(());
    }

    let attacker = *war
        .attackers
        .iter()
        .next()
        .ok_or("war_error.peace.no_attacker")?;
    let defender = *war
        .defenders
        .iter()
        .next()
        .ok_or("war_error.peace.no_defender")?;

    let target_state_id = war
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied());

    // 1. 講和条件に応じた領土移転と戦争ステータスの更新
    let (final_status, winner) = match term {
        PeaceTerm::CedeWarGoalRegion => {
            if let Some(target_id) = target_state_id {
                let _ = state_registry.transfer_region_ownership(target_id, attacker);
            }
            (WarStatus::AttackerVictory, Some(attacker))
        }
        PeaceTerm::AttackerConcedes => (WarStatus::DefenderVictory, Some(defender)),
        PeaceTerm::WhitePeace => (WarStatus::WhitePeace, None),
    };

    // 2. 占領状態の正常化（割譲されなかった陸上地域の支配権を現在の所有国へ戻す）
    for state in state_registry.states.iter_mut() {
        if !state.is_sea
            && (state.war_id == Some(war_id) || state.controller_country.is_some())
            && (state.owner_country_id == attacker || state.owner_country_id == defender)
        {
            state.controller_country = Some(state.owner_country_id);
            state.occupation_progress = 0.0;
            state.original_owner = None;
            state.war_id = None;
        }
    }

    // 3. 進行中戦闘の中止と陸軍状態のクリア
    for battle in battle_registry.battles.values_mut() {
        if battle.war_id == war_id && battle.status == BattleStatus::Ongoing {
            battle.status = BattleStatus::Cancelled;

            for division_id in battle
                .attacker_division_ids
                .iter()
                .chain(battle.defender_division_ids.iter())
            {
                if let Some(d) = military_registry
                    .divisions
                    .get_mut(division_id)
                    .filter(|d| d.status == DivisionStatus::Fighting)
                {
                    d.status = DivisionStatus::Idle;
                }
            }
        }
    }

    // 4. 対応する前線・作戦命令・部隊割り当ての安全な削除
    if let Some(fl) = frontline_registry.get_frontline_for_war(war_id).cloned() {
        frontline_registry.remove_frontline(fl.frontline_id, military_registry);
    }

    // 5. 敵国領残存陸軍の帰還
    relocate_hostile_territory_divisions(state_registry, military_registry);

    // 6. 5年間 (1825日) の休戦の作成
    let curr_date = GameDate::from_string(current_date_str).unwrap_or_default();
    let truce_end_date = curr_date.add_days(1825).display();
    diplomacy_registry.set_truce(attacker, defender, truce_end_date.clone());

    // 7. 戦争データの終了情報更新
    let war = war_registry.wars.get_mut(&war_id).unwrap();
    war.status = final_status;
    war.winner = winner;
    war.end_date = Some(current_date_str.to_string());
    war.end_reason = Some(end_reason.to_string());
    war.applied_terms.push(term.display_name().to_string());

    let start_date = GameDate::from_string(&war.start_date).unwrap_or_default();
    war.duration_days = curr_date.days_since(&start_date).max(0) as u32;

    Ok(())
}
