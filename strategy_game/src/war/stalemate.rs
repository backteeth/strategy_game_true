//! P29-003: 長期停滞したAI戦争の白紙講和(P29-003-DIAG-001 の推奨 E1)。
//!
//! AI には和平の提案・受諾経路がなく、戦争は「目標州の占領による降伏」「国境消失(P28-007)」
//! 「国家消滅」でしか終わらない。攻勢開始が国家備蓄のストック判定(70日分)であるため、
//! 月産が月需要を下回る国家は攻勢できず、防御側が無力(師団 0)でも戦争が終わらない
//! (ISS-1・ISS-2、診断報告書 §4.4)。本モジュールは、その停滞した戦争だけを、既存の
//! 白紙講和 API(`execute_peace_settlement`、`settle_capitulation` 経由)で終結させる。
//!
//! ## 停滞の定義(すべてを満たす戦争だけが対象)
//!
//! 1. **長期間、実際の軍事的進展がない**: 進展の署名(`progress_signature`)が
//!    [`STALEMATE_WHITE_PEACE_DAYS`] 日間変化せず、その間に進行中の戦闘もない。
//!    「進展」= 戦闘の発生・終了(戦闘の集計値)と、州の支配変更(所有者と支配者が
//!    異なる州の集合の変化)。評価処理が走ったことや前線計画の再生成は進展ではない。
//! 2. **一方の陣営に生存師団がない**(支持国を含む陣営全体で確認)。
//! 3. **他方も攻勢を開始できない**: 前線計画が Offensive でなく、軍事 AI の直近判断が
//!    `SufficientAdvantage` でなく、移動中・戦闘中・退却中・占領中の師団がない。
//! 4. **既存の和平判定を通る**: `can_accept_peace_offer`(白紙講和、開戦 30 日以上・
//!    戦勝点条件)を、陣営の代表国どうしの提案として通す。
//! 5. **プレイヤーが主要国・支持国のいずれとしても参加していない**戦争のみ(AI 同士)。
//!
//! 停滞期間(365 日)はゲーム内日数(`GameDate::days_since` の暦近似)で、今回導入する
//! 設計値である。診断報告書に具体的な推奨値がなかったため、初期案の 365 日を採用した
//! (理由: ISS-1・ISS-2 のいずれも 16,000 日・9,400 日の継続で、1 年は「短期の膠着」と
//! 区別できる十分長い期間であり、通常の短期戦争[数日〜数百日]を巻き込みにくい)。国別・WarId 別の条件は持たない。
//!
//! ## 永続化
//!
//! 停滞期間の起点は [`WarStalemateRecord`] として `WarRegistry` に保持し、Save V1 の
//! `SavedWarRegistry.stalemate` へ追加フィールド(`#[serde(default)]`)として保存する。
//! 古い Save には存在せず、その場合は記録なしとして、ロード後の最初の評価日から計測
//! する(ロード直後に期間満了とは見なさない)。

use crate::common::{CountryId, WarId};
use crate::country::CountryRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};
use crate::war::frontline::{FrontlineRegistry, FrontlineStance};
use crate::war::military_ai::{MilitaryAiDecisionReason, MilitaryAiRegistry};
use crate::war::peace::{PeaceOffer, PeaceTerm, can_accept_peace_offer};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 軍事的進展が無い期間(ゲーム内日数)がこの値以上で、停滞とみなす。
pub const STALEMATE_WHITE_PEACE_DAYS: i32 = 365;

/// 停滞による白紙講和の`end_reason`(P28-007 の`"Frontline Unreachable"`等と区別する)。
pub const STALEMATE_END_REASON: &str = "Stalemate White Peace";

/// 1 つの戦争の停滞期間の計測状態(永続化対象)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarStalemateRecord {
    /// 最後に軍事的進展(署名の変化・戦闘)を観測した日(`YYYY/MM/DD`)。
    pub last_progress_date: String,
    /// その時点の進展の署名。
    pub signature: u64,
}

/// FNV-1a(64bit)。標準ライブラリのハッシャは実行間で値が固定される保証がないため、
/// Save へ保存する署名には、手書きの決定論的ハッシュを使う。
fn fnv1a(values: impl IntoIterator<Item = u64>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in values {
        for b in v.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

fn participants(war: &War) -> HashSet<CountryId> {
    war.attackers.iter().chain(war.defenders.iter()).copied().collect()
}

/// この戦争の「軍事的進展の署名」: 戦闘の集計値と、参加国が関わる州の支配変更。
/// 署名が変われば進展があったとみなす。
pub fn progress_signature(war: &War, state_registry: &StateRegistry) -> u64 {
    let who = participants(war);
    let mut controls: Vec<(u64, u64, u64)> = state_registry
        .states
        .iter()
        .filter(|s| {
            let owner = s.owner_country_id;
            let controller = s.controller();
            owner != controller && (who.contains(&owner) || who.contains(&controller))
        })
        .map(|s| {
            (
                s.id.0 as u64,
                s.owner_country_id.0 as u64,
                s.controller().0 as u64,
            )
        })
        .collect();
    controls.sort();
    let mut values: Vec<u64> = vec![
        war.won_attacker_battles as u64,
        war.won_defender_battles as u64,
        war.processed_battle_ids.len() as u64,
        war.occupied_states.len() as u64,
        controls.len() as u64,
    ];
    for (s, o, c) in controls {
        values.extend([s, o, c]);
    }
    fnv1a(values)
}

fn side_has_living_division(side: &HashSet<CountryId>, military: &MilitaryRegistry) -> bool {
    military.divisions.values().any(|d| {
        side.contains(&d.owner) && d.manpower > 0 && d.status != DivisionStatus::Destroyed
    })
}

/// `armed`側(生存師団を持つ陣営)が、いま攻勢を開始・継続できる(または動いている)か。
fn armed_side_is_active(
    war: &War,
    armed: &HashSet<CountryId>,
    military: &MilitaryRegistry,
    frontlines: &FrontlineRegistry,
    ai: Option<&MilitaryAiRegistry>,
) -> bool {
    // 移動中・戦闘中・退却中・占領中の師団がある(進行中の攻撃・戦闘を妨げない)。
    let moving = military.divisions.values().any(|d| {
        armed.contains(&d.owner)
            && d.manpower > 0
            && matches!(
                d.status,
                DivisionStatus::Moving
                    | DivisionStatus::Fighting
                    | DivisionStatus::Retreating
                    | DivisionStatus::Occupying
            )
    });
    if moving {
        return true;
    }
    let frontline_id = frontlines.get_frontline_for_war(war.id).map(|f| f.frontline_id);
    for &country in armed {
        // 前線計画が攻勢姿勢。
        if let Some(fid) = frontline_id
            && frontlines
                .plans
                .get(&(fid, country))
                .is_some_and(|p| p.stance == FrontlineStance::Offensive)
        {
            return true;
        }
        // 軍事 AI の直近判断が「攻勢可能」。
        if ai
            .and_then(|r| r.ai_states.get(&country))
            .is_some_and(|s| s.last_decision_reason == MilitaryAiDecisionReason::SufficientAdvantage)
        {
            return true;
        }
    }
    false
}

/// 停滞条件(上記 2・3)。進展の無い期間は呼び出し側が確認済みである前提。
pub fn is_one_side_powerless_and_other_cannot_attack(
    war: &War,
    military: &MilitaryRegistry,
    frontlines: &FrontlineRegistry,
    ai: Option<&MilitaryAiRegistry>,
) -> bool {
    let attackers: HashSet<CountryId> = war.attackers.iter().copied().collect();
    let defenders: HashSet<CountryId> = war.defenders.iter().copied().collect();
    let attackers_armed = side_has_living_division(&attackers, military);
    let defenders_armed = side_has_living_division(&defenders, military);
    // 両陣営に師団が存在する戦争は対象外。
    if attackers_armed && defenders_armed {
        return false;
    }
    // 武装している側(あれば)が、攻勢を開始・継続できる状態でない。
    if attackers_armed && armed_side_is_active(war, &attackers, military, frontlines, ai) {
        return false;
    }
    if defenders_armed && armed_side_is_active(war, &defenders, military, frontlines, ai) {
        return false;
    }
    true
}

/// 毎日 1 回、全 Active War の停滞期間を更新し、白紙講和にすべき戦争の ID(昇順)を返す。
///
/// - 記録の無い戦争(新規・古い Save)は、今日を起点として記録する(即時満了にしない)。
/// - 進展があった(署名の変化・進行中の戦闘)日に起点をリセットする。
/// - Active でなくなった戦争の記録は削除する。
/// - プレイヤーが参加する戦争・`war_goals`が空の戦争は対象外(記録も持たない)。
///
/// 講和の実行(`settle_capitulation`)は呼び出し側が行う。
#[allow(clippy::too_many_arguments)]
pub fn update_stalemate_records(
    war_registry: &mut WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    battle_registry: &BattleRegistry,
    frontline_registry: &FrontlineRegistry,
    country_registry: &CountryRegistry,
    ai_registry: Option<&MilitaryAiRegistry>,
    player: Option<CountryId>,
    today: &str,
) -> Vec<WarId> {
    let Some(today_date) = crate::app::time::GameDate::from_string(today) else {
        return Vec::new();
    };

    // 非 Active の記録を整理する。
    let active_ids: HashSet<WarId> = war_registry
        .wars
        .values()
        .filter(|w| w.status == WarStatus::Active)
        .map(|w| w.id)
        .collect();
    war_registry
        .stalemate_records
        .retain(|id, _| active_ids.contains(id));

    let mut ids: Vec<WarId> = active_ids.into_iter().collect();
    ids.sort_by_key(|id| id.0);

    let mut to_settle = Vec::new();
    for id in ids {
        let (eligible, signature, ongoing_battle) = {
            let war = &war_registry.wars[&id];
            let player_involved = player.is_some_and(|p| war.is_participant(p));
            let eligible = !war.war_goals.is_empty() && !player_involved;
            let ongoing = battle_registry
                .battles
                .values()
                .any(|b| b.war_id == id && b.status == BattleStatus::Ongoing);
            (eligible, progress_signature(war, state_registry), ongoing)
        };
        if !eligible {
            war_registry.stalemate_records.remove(&id);
            continue;
        }

        let needs_reset = match war_registry.stalemate_records.get(&id) {
            None => true,
            Some(rec) => rec.signature != signature || ongoing_battle,
        };
        if needs_reset {
            war_registry.stalemate_records.insert(
                id,
                WarStalemateRecord {
                    last_progress_date: today.to_string(),
                    signature,
                },
            );
            continue;
        }

        let rec = &war_registry.stalemate_records[&id];
        let Some(last) = crate::app::time::GameDate::from_string(&rec.last_progress_date) else {
            // 壊れた記録は今日から再計測する。
            war_registry.stalemate_records.insert(
                id,
                WarStalemateRecord {
                    last_progress_date: today.to_string(),
                    signature,
                },
            );
            continue;
        };
        if today_date.days_since(&last) < STALEMATE_WHITE_PEACE_DAYS {
            continue;
        }

        let war = &war_registry.wars[&id];
        if !is_one_side_powerless_and_other_cannot_attack(
            war,
            military_registry,
            frontline_registry,
            ai_registry,
        ) {
            continue;
        }
        // 陣営の代表国どうしの白紙講和提案として、既存の受諾判定を通す。
        let offer = PeaceOffer {
            war_id: id,
            proposer_country_id: war.primary_attacker_id(),
            recipient_country_id: war.primary_defender_id(),
            term: PeaceTerm::WhitePeace,
            created_date: today.to_string(),
        };
        if can_accept_peace_offer(
            &offer,
            war_registry,
            Some(state_registry),
            Some(military_registry),
            Some(country_registry),
            today,
        )
        .is_ok()
        {
            to_settle.push(id);
        }
    }
    to_settle
}

/// 講和などで Active でなくなった戦争の停滞記録を削除する(Save へ残さない)。
pub fn prune_inactive_records(war_registry: &mut WarRegistry) {
    let active: HashSet<WarId> = war_registry
        .wars
        .values()
        .filter(|w| w.status == WarStatus::Active)
        .map(|w| w.id)
        .collect();
    war_registry
        .stalemate_records
        .retain(|id, _| active.contains(id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{DivisionDefinitionId, DivisionId, FrontlineId, StateId};
    use crate::diplomacy::crisis::{WarGoal, WarGoalType};
    use crate::military::data::{Division, DivisionSize, DivisionType};
    use crate::state::data::StateData;
    use crate::war::frontline::{Frontline, FrontlinePlan};
    use crate::war::military_ai::MilitaryAiState;

    const ATT: CountryId = CountryId(0);
    const ATT_SUPPORT: CountryId = CountryId(2);
    const DEF: CountryId = CountryId(1);
    const WID: WarId = WarId(7);

    fn war(id: WarId, start: &str) -> War {
        War {
            id,
            name: "stalemate test".to_string(),
            attackers: [ATT, ATT_SUPPORT].into_iter().collect(),
            defenders: [DEF].into_iter().collect(),
            primary_attacker: Some(ATT),
            primary_defender: Some(DEF),
            source_crisis_id: None,
            war_goals: vec![WarGoal {
                attacker: ATT,
                defender: DEF,
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(1)],
                base_peace_cost: 20.0,
                international_concern: 10.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: start.to_string(),
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
        }
    }

    fn division(id: usize, owner: CountryId, status: DivisionStatus) -> Division {
        Division {
            id: DivisionId(id),
            owner,
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(0),
            destination: None,
            current_path: Vec::new(),
            target_state: None,
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 1000.0,
            max_equipment: 1000.0,
            organization: 50.0,
            max_organization: 50.0,
            morale: 50.0,
            max_morale: 50.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status,
            def_id: DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    struct Env {
        wars: WarRegistry,
        states: StateRegistry,
        military: MilitaryRegistry,
        battles: BattleRegistry,
        frontlines: FrontlineRegistry,
        countries: CountryRegistry,
        ai: MilitaryAiRegistry,
    }

    fn env() -> Env {
        let mut wars = WarRegistry::default();
        wars.wars.insert(WID, war(WID, "1800/01/01"));
        let states = StateRegistry::build(vec![
            StateData {
                id: StateId(0),
                owner_country_id: ATT,
                ..Default::default()
            },
            StateData {
                id: StateId(1),
                owner_country_id: DEF,
                ..Default::default()
            },
        ]);
        let countries = CountryRegistry {
            countries: [ATT, DEF, ATT_SUPPORT]
                .into_iter()
                .map(|id| crate::country::CountryData {
                    id,
                    ..Default::default()
                })
                .collect(),
        };
        Env {
            wars,
            states,
            military: MilitaryRegistry::default(),
            battles: BattleRegistry::default(),
            frontlines: FrontlineRegistry::default(),
            countries,
            ai: MilitaryAiRegistry::default(),
        }
    }

    fn run(e: &mut Env, today: &str, player: Option<CountryId>) -> Vec<WarId> {
        update_stalemate_records(
            &mut e.wars,
            &e.states,
            &e.military,
            &e.battles,
            &e.frontlines,
            &e.countries,
            Some(&e.ai),
            player,
            today,
        )
    }

    /// 攻撃側(主要国)だけが師団を持つ(防御側は師団 0)。
    fn arm_attacker(e: &mut Env, status: DivisionStatus) {
        e.military.add_division(division(0, ATT, status));
    }

    #[test]
    fn a_war_without_a_record_starts_counting_today_and_is_never_settled_on_that_day() {
        let mut e = env();
        // 開戦から 1000 日経っていても、記録が無い(新規・古い Save)日は期間満了とみなさない。
        assert!(run(&mut e, "1802/10/01", None).is_empty());
        assert_eq!(e.wars.stalemate_records[&WID].last_progress_date, "1802/10/01");
        assert!(run(&mut e, "1803/09/26", None).is_empty(), "364 日");
        assert_eq!(run(&mut e, "1803/10/01", None), vec![WID], "365 日");
    }

    #[test]
    fn period_is_exactly_the_configured_number_of_days() {
        let mut e = env();
        assert!(run(&mut e, "1800/01/01", None).is_empty());
        assert!(run(&mut e, "1800/12/30", None).is_empty());
        assert_eq!(run(&mut e, "1801/01/01", None), vec![WID]);
        assert_eq!(STALEMATE_WHITE_PEACE_DAYS, 365);
    }

    #[test]
    fn a_state_control_change_resets_the_period() {
        let mut e = env();
        run(&mut e, "1800/01/01", None);
        e.states.get_mut(StateId(1)).unwrap().controller_country = Some(ATT);
        assert!(run(&mut e, "1800/12/01", None).is_empty(), "進展: 起点が 1800/12/01 になる");
        assert!(run(&mut e, "1801/01/01", None).is_empty(), "起点から 30 日");
        assert!(run(&mut e, "1801/11/30", None).is_empty());
        assert_eq!(run(&mut e, "1801/12/01", None), vec![WID]);
    }

    #[test]
    fn a_battle_result_resets_the_period_and_an_ongoing_battle_resets_it_every_day() {
        let mut e = env();
        run(&mut e, "1800/01/01", None);
        e.wars.wars.get_mut(&WID).unwrap().won_attacker_battles = 1;
        assert!(run(&mut e, "1800/12/31", None).is_empty());
        assert!(run(&mut e, "1801/12/30", None).is_empty());
        assert_eq!(run(&mut e, "1801/12/31", None), vec![WID]);

        let mut e = env();
        run(&mut e, "1800/01/01", None);
        let battle = crate::military::battle::Battle {
            id: crate::common::BattleId(0),
            war_id: WID,
            state_id: StateId(1),
            attacker_country: ATT,
            defender_country: DEF,
            attacker_division_ids: vec![],
            defender_division_ids: vec![],
            attacker_origins: Default::default(),
            start_date: "1800/12/01".to_string(),
            elapsed_days: 0,
            status: BattleStatus::Ongoing,
        };
        e.battles.battles.insert(battle.id, battle);
        // 進行中の戦闘がある間は、期間を超えていても毎日リセットされる。
        assert!(run(&mut e, "1802/06/01", None).is_empty());
        assert!(run(&mut e, "1802/06/02", None).is_empty());
        e.battles.battles.clear();
        assert!(run(&mut e, "1802/06/03", None).is_empty(), "戦闘終了日が新しい起点");
        assert_eq!(run(&mut e, "1803/06/03", None), vec![WID]);
    }

    #[test]
    fn a_war_where_both_sides_have_divisions_is_excluded() {
        let mut e = env();
        arm_attacker(&mut e, DivisionStatus::Idle);
        e.military.add_division(division(1, DEF, DivisionStatus::Idle));
        run(&mut e, "1800/01/01", None);
        assert!(run(&mut e, "1805/01/01", None).is_empty());
    }

    #[test]
    fn a_supporter_with_a_division_means_the_side_is_not_powerless() {
        let mut e = env();
        // 防御側(DEF)は師団 0。攻撃側は支持国(ATT_SUPPORT)だけが師団を持つが、攻勢できる状態
        // ではない(Idle・計画なし)ので、終結する。
        e.military.add_division(division(0, ATT_SUPPORT, DivisionStatus::Idle));
        run(&mut e, "1800/01/01", None);
        assert_eq!(run(&mut e, "1801/01/01", None), vec![WID]);
        // 防御側にも支持国の師団が加わると、両陣営が武装しているため対象外。
        let mut e = env();
        e.wars.wars.get_mut(&WID).unwrap().defenders.insert(CountryId(3));
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        e.military.add_division(division(1, CountryId(3), DivisionStatus::Idle));
        run(&mut e, "1800/01/01", None);
        assert!(run(&mut e, "1805/01/01", None).is_empty());
    }

    #[test]
    fn an_armed_side_that_can_attack_or_is_moving_is_not_stalled() {
        for status in [
            DivisionStatus::Moving,
            DivisionStatus::Fighting,
            DivisionStatus::Retreating,
            DivisionStatus::Occupying,
        ] {
            let mut e = env();
            arm_attacker(&mut e, status);
            run(&mut e, "1800/01/01", None);
            assert!(run(&mut e, "1803/01/01", None).is_empty(), "{status:?}");
        }
        // 攻勢姿勢の前線計画。
        let mut e = env();
        arm_attacker(&mut e, DivisionStatus::Idle);
        e.frontlines.frontlines.insert(
            FrontlineId(0),
            Frontline {
                frontline_id: FrontlineId(0),
                war_id: WID,
                attacker_country_id: ATT,
                defender_country_id: DEF,
                attacker_front_regions: vec![StateId(0)],
                defender_front_regions: vec![StateId(1)],
                border_region_pairs: vec![(StateId(0), StateId(1))],
            },
        );
        let mut plan = FrontlinePlan::new(FrontlineId(0), ATT);
        plan.stance = FrontlineStance::Offensive;
        e.frontlines.plans.insert((FrontlineId(0), ATT), plan);
        run(&mut e, "1800/01/01", None);
        assert!(run(&mut e, "1803/01/01", None).is_empty(), "Offensive 計画");
        // 防御姿勢・攻勢できない(SupplyShortage)なら対象。
        e.frontlines
            .plans
            .get_mut(&(FrontlineId(0), ATT))
            .unwrap()
            .stance = FrontlineStance::Defend;
        let mut st = MilitaryAiState::new(ATT);
        st.last_decision_reason = MilitaryAiDecisionReason::SupplyShortage;
        e.ai.ai_states.insert(ATT, st);
        assert_eq!(run(&mut e, "1803/01/02", None), vec![WID]);
        // 軍事 AI の直近判断が攻勢可能(SufficientAdvantage)なら対象外。
        e.ai.ai_states.get_mut(&ATT).unwrap().last_decision_reason =
            MilitaryAiDecisionReason::SufficientAdvantage;
        assert!(run(&mut e, "1803/01/03", None).is_empty());
    }

    #[test]
    fn wars_with_a_player_participant_are_excluded_and_not_tracked() {
        for player in [ATT, DEF, ATT_SUPPORT] {
            let mut e = env();
            run(&mut e, "1800/01/01", Some(player));
            assert!(e.wars.stalemate_records.is_empty(), "{player:?}");
            assert!(run(&mut e, "1810/01/01", Some(player)).is_empty());
        }
        // プレイヤーが無関係な国なら対象。
        let mut e = env();
        run(&mut e, "1800/01/01", Some(CountryId(9)));
        assert_eq!(run(&mut e, "1801/01/01", Some(CountryId(9))), vec![WID]);
    }

    #[test]
    fn a_war_without_goals_is_excluded() {
        let mut e = env();
        e.wars.wars.get_mut(&WID).unwrap().war_goals.clear();
        run(&mut e, "1800/01/01", None);
        assert!(run(&mut e, "1810/01/01", None).is_empty());
        assert!(e.wars.stalemate_records.is_empty());
    }

    #[test]
    fn only_the_stalled_war_is_returned_and_ended_wars_lose_their_record() {
        let mut e = env();
        let other = WarId(8);
        let mut w = war(other, "1800/01/01");
        w.attackers = [CountryId(5)].into_iter().collect();
        w.defenders = [CountryId(6)].into_iter().collect();
        w.primary_attacker = Some(CountryId(5));
        w.primary_defender = Some(CountryId(6));
        e.wars.wars.insert(other, w);
        e.military
            .add_division(division(0, CountryId(5), DivisionStatus::Moving));
        run(&mut e, "1800/01/01", None);
        assert_eq!(
            run(&mut e, "1801/01/01", None),
            vec![WID],
            "移動中の師団を持つ戦争は対象外"
        );
        e.wars.wars.get_mut(&WID).unwrap().status = WarStatus::AttackerVictory;
        run(&mut e, "1801/01/02", None);
        assert!(!e.wars.stalemate_records.contains_key(&WID));
        assert!(e.wars.stalemate_records.contains_key(&other));
    }

    #[test]
    fn the_existing_white_peace_acceptance_check_still_applies() {
        let mut e = env();
        e.wars.wars.get_mut(&WID).unwrap().war_score = -50.0;
        run(&mut e, "1800/01/01", None);
        assert!(
            run(&mut e, "1802/01/01", None).is_empty(),
            "戦勝点が -10 未満の戦争は、既存の白紙講和の受諾判定を通らない"
        );
    }

    #[test]
    fn the_progress_signature_is_deterministic_and_changes_only_with_progress() {
        let e = env();
        let w = &e.wars.wars[&WID];
        let s0 = progress_signature(w, &e.states);
        assert_eq!(s0, progress_signature(w, &e.states));
        let mut e2 = env();
        e2.states.get_mut(StateId(0)).unwrap().controller_country = Some(DEF);
        assert_ne!(s0, progress_signature(&e2.wars.wars[&WID], &e2.states));
        // 評価の実行や前線計画の再生成は署名に影響しない(入力に含まれない)。
        let mut e3 = env();
        e3.frontlines
            .plans
            .insert((FrontlineId(0), ATT), FrontlinePlan::new(FrontlineId(0), ATT));
        assert_eq!(s0, progress_signature(&e3.wars.wars[&WID], &e3.states));
    }

    #[test]
    fn an_old_save_without_the_stalemate_field_restores_as_an_empty_record_set() {
        // 旧 Save(`stalemate`フィールドなし)は、追加フィールドの既定値(空)として復元される。
        let old: crate::save::dto::SavedWarRegistry =
            ron::from_str("(wars: {}, next_id: 3)").expect("old save format must parse");
        assert!(old.stalemate.is_empty());
        assert_eq!(old.next_id, 3);

        // 記録は RON で往復できる。
        let mut saved = crate::save::dto::SavedWarRegistry::default();
        saved.stalemate.insert(
            WarId(4),
            WarStalemateRecord {
                last_progress_date: "1800/06/14".to_string(),
                signature: u64::MAX - 7,
            },
        );
        let text = ron::to_string(&saved).unwrap();
        let back: crate::save::dto::SavedWarRegistry = ron::from_str(&text).unwrap();
        assert_eq!(back.stalemate, saved.stalemate);
    }
}
