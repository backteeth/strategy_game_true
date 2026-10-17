//! P29-003: 長期停滞したAI戦争の白紙講和(P29-003-DIAG-001 の推奨 E1)。
//!
//! AI には和平の提案・受諾経路がなく、戦争は「目標州の占領による降伏」「国境消失(P28-007)」
//! 「国家消滅」でしか終わらない。攻勢開始が国家備蓄のストック判定(70日分)であるため、
//! 月産が月需要を下回る国家は攻勢できず、防御側が無力(師団 0)でも戦争が終わらない
//! (ISS-1・ISS-2、診断報告書 §4.4)。本モジュールは、その停滞した戦争だけを、既存の
//! 白紙講和 API(`execute_peace_settlement`、`settle_capitulation` 経由)で終結させる。
//!
//! ## M1(P31-002): 両陣営に師団がある停滞戦争の合意白紙講和
//!
//! E1 は「一方の陣営に師団がない」戦争だけが対象で、双方が武装した戦力拮抗の停滞は終わらなかった
//! (P31-001 の診断)。M1 は E1 と**同じ停滞記録(`WarStalemateRecord`)を共有する別の分岐**で、
//! 新しい永続状態を持たない。E1 の条件・期間・結果は変更しない(E1 の判定関数は両陣営が武装している
//! 戦争で常に`false`を返すため、E1 と M1 は排他的)。
//!
//! M1 の成立条件(すべて): E1 と同じ適格判定(AI 同士・戦争目標あり)、進展のない期間が
//! [`STALEMATE_MUTUAL_WHITE_PEACE_DAYS`](730 日)以上、両陣営(支持国を含む)に生存師団がある、
//! 生存師団を持つ全参加国に軍事 AI の判断記録がある(未観測を「意思なし」と扱わない)、双方に攻勢の意思が
//! ない([`side_has_attack_intent`])、この戦争に帰属する占領・戦争目標の達成がない
//! ([`war_has_occupation_or_goal_achieved`])、既存の白紙講和の受諾判定が両方向で通る。
//! 終結理由は[`MUTUAL_STALEMATE_END_REASON`]で E1 と区別する。
//!
//! ## 停滞の定義(E1: すべてを満たす戦争だけが対象)
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

/// P31-002(M1): 両陣営に師団がある戦争の合意白紙講和に必要な、軍事的進展のない期間(ゲーム内日数)。
/// E1 の 2 倍。根拠は「判断→建設→生産→備蓄の橋渡し→募兵」の AI の回復の連鎖の 2 回分で、
/// 武装した双方が戦力を回復・増強できる戦争に、少なくとも 2 回の機会が結果を生まなかったことを要求する
/// (P31-001 §5-3)。標準シナリオの戦争件数に合わせた値ではない。
pub const STALEMATE_MUTUAL_WHITE_PEACE_DAYS: i32 = 730;

/// M1(双方武装停滞の合意白紙講和)の`end_reason`。E1 の[`STALEMATE_END_REASON`]と区別する。
pub const MUTUAL_STALEMATE_END_REASON: &str = "Mutual Stalemate White Peace";

/// 停滞による白紙講和の種別(終結理由の識別に使う)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalemateKind {
    /// E1(P29-003): 一方の陣営に師団がない。
    OneSidePowerless,
    /// M1(P31-002): 両陣営に師団があり、双方に攻勢の意思がない。
    MutualArmed,
}

impl StalemateKind {
    pub fn end_reason(self) -> &'static str {
        match self {
            StalemateKind::OneSidePowerless => STALEMATE_END_REASON,
            StalemateKind::MutualArmed => MUTUAL_STALEMATE_END_REASON,
        }
    }
}

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

/// M1: `side`のいずれかの国が、いま攻勢の意思を持つか。E1 の`armed_side_is_active`
/// (移動中・戦闘中・退却中・占領中の師団、Offensive 計画、`SufficientAdvantage` 判断)に加えて、
/// 直近判断が`SupplyShortage`で、記録済みの戦力比が攻勢の開始条件(own × 1000 ≥ enemy × 1100)を
/// 満たす国も、補給が回復すれば攻勢に移る陣営として「意思あり」とみなす。
/// 開始条件の倍率は`evaluate_military_ai_decision`と同じ値を読むだけで、変更しない。
fn side_has_attack_intent(
    war: &War,
    side: &HashSet<CountryId>,
    military: &MilitaryRegistry,
    frontlines: &FrontlineRegistry,
    ai: &MilitaryAiRegistry,
) -> bool {
    if armed_side_is_active(war, side, military, frontlines, Some(ai)) {
        return true;
    }
    side.iter().any(|c| {
        ai.ai_states.get(c).is_some_and(|s| {
            s.last_decision_reason == MilitaryAiDecisionReason::SupplyShortage
                && (s.estimated_own_power as u128) * 1000 >= (s.estimated_enemy_power as u128) * 1100
        })
    })
}

/// M1: 生存師団を持つ参加国のすべてに、軍事 AI の判断記録があるか。記録がない、または既定値の
/// `NoActiveWar`(まだ一度も評価されていない)の国は、攻勢の意思を判断できないため保留にする
/// (「未観測」を根拠なく「意思なし」と扱わない)。師団を持たない参加国は攻勢できないので対象外。
fn all_armed_participants_have_a_decision(
    war: &War,
    military: &MilitaryRegistry,
    ai: &MilitaryAiRegistry,
) -> bool {
    participants(war).iter().all(|c| {
        let armed = military
            .divisions
            .values()
            .any(|d| d.owner == *c && d.manpower > 0 && d.status != DivisionStatus::Destroyed);
        !armed
            || ai
                .ai_states
                .get(c)
                .is_some_and(|s| s.last_decision_reason != MilitaryAiDecisionReason::NoActiveWar)
    })
}

/// M1: この戦争に帰属する占領または戦争目標の達成があるか。
///
/// 占領モデル(`military::invasion::occupy_state`は`controller_country`だけを書き、`state.war_id`は
/// 本番で設定されない)に従い、P28-008 の終結時の正規化と同じ帰属規則を使う: 陸の州で、所有国と
/// 支配国が異なり、かつ両者がこの戦争の参加国である。加えて、`war.occupied_states`が空でないこと、
/// 戦争目標の州を攻撃側が(所有せず)支配していることも「成果あり」とする。
/// **通常の領土要求(戦争目標があるだけ)は係争に数えない**。
fn war_has_occupation_or_goal_achieved(war: &War, state_registry: &StateRegistry) -> bool {
    if !war.occupied_states.is_empty() {
        return true;
    }
    let who = participants(war);
    let occupation = state_registry.states.iter().any(|s| {
        !s.is_sea
            && s.owner_country_id != s.controller()
            && who.contains(&s.owner_country_id)
            && who.contains(&s.controller())
    });
    if occupation {
        return true;
    }
    war.war_goals.iter().flat_map(|g| g.target_states.iter()).any(|id| {
        state_registry.get(*id).is_some_and(|s| {
            !war.attackers.contains(&s.owner_country_id) && war.attackers.contains(&s.controller())
        })
    })
}

/// M1: 双方の受諾(攻撃側→防御側、防御側→攻撃側のどちらの白紙講和提案も受諾される)の合成。
/// 片方向でも失敗すれば不成立。
fn agreed_by_both(
    attacker: CountryId,
    defender: CountryId,
    accepts: impl Fn(CountryId, CountryId) -> bool,
) -> bool {
    accepts(attacker, defender) && accepts(defender, attacker)
}

/// M1(P31-002)の停滞条件(進展のない期間は呼び出し側が確認済みの前提)。
/// 両陣営(支持国を含む)に生存師団があり、双方に攻勢の意思がなく、判断が観測済みで、
/// 占領・戦争目標の達成がない。受諾判定は呼び出し側が行う。
pub fn is_mutual_stalemate(
    war: &War,
    state_registry: &StateRegistry,
    military: &MilitaryRegistry,
    frontlines: &FrontlineRegistry,
    ai: Option<&MilitaryAiRegistry>,
) -> bool {
    let Some(ai) = ai else { return false };
    let attackers: HashSet<CountryId> = war.attackers.iter().copied().collect();
    let defenders: HashSet<CountryId> = war.defenders.iter().copied().collect();
    if !(side_has_living_division(&attackers, military)
        && side_has_living_division(&defenders, military))
    {
        return false;
    }
    if !all_armed_participants_have_a_decision(war, military, ai) {
        return false;
    }
    if side_has_attack_intent(war, &attackers, military, frontlines, ai)
        || side_has_attack_intent(war, &defenders, military, frontlines, ai)
    {
        return false;
    }
    !war_has_occupation_or_goal_achieved(war, state_registry)
}

/// 毎日 1 回、全 Active War の停滞期間を更新し、白紙講和にすべき戦争の ID(昇順)を返す。
/// E1・M1 のどちらで終結するかは[`update_stalemate_records_with_kind`]を参照(本関数は ID のみ)。
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
    update_stalemate_records_with_kind(
        war_registry,
        state_registry,
        military_registry,
        battle_registry,
        frontline_registry,
        country_registry,
        ai_registry,
        player,
        today,
    )
    .into_iter()
    .map(|(id, _)| id)
    .collect()
}

/// [`update_stalemate_records`]と同じ評価を行い、終結する戦争を種別(E1 / M1)つきで返す。
///
/// 評価順序(戦争ごと): 適格判定(AI のみ・戦争目標あり)→ 進展の署名・進行中の戦闘による起点の
/// 更新(E1・M1 で共通の記録)→ 365 日以上で E1(一方が無力)を評価 → E1 でなければ 730 日以上で
/// M1(両陣営が武装)を評価 → 既存の白紙講和の受諾判定(M1 は両方向)。E1 の条件・期間・結果は
/// M1 の導入前と同一(M1 は E1 が不成立の戦争にだけ働く)。
#[allow(clippy::too_many_arguments)]
pub fn update_stalemate_records_with_kind(
    war_registry: &mut WarRegistry,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    battle_registry: &BattleRegistry,
    frontline_registry: &FrontlineRegistry,
    country_registry: &CountryRegistry,
    ai_registry: Option<&MilitaryAiRegistry>,
    player: Option<CountryId>,
    today: &str,
) -> Vec<(WarId, StalemateKind)> {
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
        let days = today_date.days_since(&last);
        if days < STALEMATE_WHITE_PEACE_DAYS {
            continue;
        }

        let war = &war_registry.wars[&id];
        // E1(一方が無力)。両陣営に師団がある戦争では常に false なので、M1 とは排他的。
        let kind = if is_one_side_powerless_and_other_cannot_attack(
            war,
            military_registry,
            frontline_registry,
            ai_registry,
        ) {
            StalemateKind::OneSidePowerless
        } else if days >= STALEMATE_MUTUAL_WHITE_PEACE_DAYS
            && is_mutual_stalemate(
                war,
                state_registry,
                military_registry,
                frontline_registry,
                ai_registry,
            )
        {
            StalemateKind::MutualArmed
        } else {
            continue;
        };
        // 陣営の代表国どうしの白紙講和提案として、既存の受諾判定を通す。M1 は双方の受諾が
        // 必要なので、両方向で通ることを要求する(E1 は従来どおり攻撃側→防御側のみ)。
        let accepts = |proposer: CountryId, recipient: CountryId| {
            let offer = PeaceOffer {
                war_id: id,
                proposer_country_id: proposer,
                recipient_country_id: recipient,
                term: PeaceTerm::WhitePeace,
                created_date: today.to_string(),
            };
            can_accept_peace_offer(
                &offer,
                war_registry,
                Some(state_registry),
                Some(military_registry),
                Some(country_registry),
                today,
            )
            .is_ok()
        };
        let (a, d) = (war.primary_attacker_id(), war.primary_defender_id());
        let agreed = match kind {
            StalemateKind::OneSidePowerless => accepts(a, d),
            StalemateKind::MutualArmed => agreed_by_both(a, d, accepts),
        };
        if agreed {
            to_settle.push((id, kind));
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
        assert!(run(&mut e, "1802/06/03", None).is_empty(), "戦闘が最後にあった日が起点");
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

    // ─── P31-002: M1(両陣営に師団がある戦争の合意白紙講和) ───────────────────────

    const START: &str = "1800/01/15";
    const DAY_729: &str = "1802/01/14";
    const DAY_730: &str = "1802/01/15";

    fn observe(e: &mut Env, c: CountryId, reason: MilitaryAiDecisionReason) {
        let mut st = MilitaryAiState::new(c);
        st.last_decision_reason = reason;
        e.ai.ai_states.insert(c, st);
    }

    /// 両陣営に Idle の師団があり、双方の軍事 AI の直近判断が攻勢に至らない。
    fn arm_both(e: &mut Env) {
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        e.military.add_division(division(1, DEF, DivisionStatus::Idle));
        observe(e, ATT, MilitaryAiDecisionReason::ComparableStrength);
        observe(e, DEF, MilitaryAiDecisionReason::EnemyStronger);
    }

    fn run_kind(e: &mut Env, today: &str, player: Option<CountryId>) -> Vec<(WarId, StalemateKind)> {
        update_stalemate_records_with_kind(
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

    /// 起点を記録した上で、730 日後(`DAY_730`)の評価結果を返す。
    fn m1_after_the_period(e: &mut Env) -> Vec<(WarId, StalemateKind)> {
        assert!(run_kind(e, START, None).is_empty());
        run_kind(e, DAY_730, None)
    }

    #[test]
    fn m1_is_not_settled_at_729_days_and_is_settled_at_730_days() {
        assert_eq!(STALEMATE_MUTUAL_WHITE_PEACE_DAYS, 730);
        let mut e = env();
        arm_both(&mut e);
        assert!(run_kind(&mut e, START, None).is_empty(), "起点の日は満了にしない");
        assert!(run_kind(&mut e, "1801/01/15", None).is_empty(), "365 日(E1 の期間)では成立しない");
        assert!(run_kind(&mut e, DAY_729, None).is_empty(), "729 日");
        assert_eq!(
            run_kind(&mut e, DAY_730, None),
            vec![(WID, StalemateKind::MutualArmed)],
            "730 日"
        );
        assert_eq!(StalemateKind::MutualArmed.end_reason(), "Mutual Stalemate White Peace");
        assert_eq!(StalemateKind::OneSidePowerless.end_reason(), "Stalemate White Peace");
    }

    #[test]
    fn m1_period_is_reset_by_a_battle_result_and_an_ongoing_battle() {
        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
        e.wars.wars.get_mut(&WID).unwrap().won_defender_battles = 1;
        assert!(run_kind(&mut e, "1800/06/15", None).is_empty(), "戦闘結果で起点が 1800/06/15 になる");
        assert!(run_kind(&mut e, "1802/06/14", None).is_empty(), "起点から 729 日");
        assert_eq!(run_kind(&mut e, "1802/06/15", None), vec![(WID, StalemateKind::MutualArmed)]);

        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
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
        assert!(run_kind(&mut e, "1803/06/15", None).is_empty(), "進行中の戦闘がある間は毎日リセット");
        e.battles.battles.clear();
        assert!(run_kind(&mut e, "1803/06/16", None).is_empty(), "戦闘が最後にあった日が起点");
        assert!(run_kind(&mut e, "1805/06/14", None).is_empty(), "起点(戦闘が最後にあった 1803/06/15)から 729 日");
        assert_eq!(run_kind(&mut e, "1805/06/15", None), vec![(WID, StalemateKind::MutualArmed)]);
    }

    #[test]
    fn m1_is_excluded_while_either_side_is_moving_fighting_retreating_or_occupying() {
        for status in [
            DivisionStatus::Moving,
            DivisionStatus::Fighting,
            DivisionStatus::Retreating,
            DivisionStatus::Occupying,
        ] {
            for owner in [ATT, DEF, ATT_SUPPORT] {
                let mut e = env();
                arm_both(&mut e);
                e.military.add_division(division(2, owner, status));
                observe(&mut e, ATT_SUPPORT, MilitaryAiDecisionReason::ComparableStrength);
                assert!(m1_after_the_period(&mut e).is_empty(), "{owner:?} {status:?}");
            }
        }
    }

    #[test]
    fn m1_is_excluded_by_an_offensive_plan_or_an_attack_intent_of_either_side_or_a_supporter() {
        let front = |e: &mut Env| {
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
        };
        // Offensive 計画(攻撃側・防御側・支持国のどれでも)。
        for commander in [ATT, DEF, ATT_SUPPORT] {
            let mut e = env();
            arm_both(&mut e);
            front(&mut e);
            let mut plan = FrontlinePlan::new(FrontlineId(0), commander);
            plan.stance = FrontlineStance::Offensive;
            e.frontlines.plans.insert((FrontlineId(0), commander), plan);
            assert!(m1_after_the_period(&mut e).is_empty(), "Offensive {commander:?}");
        }
        // 防御姿勢なら対象。
        let mut e = env();
        arm_both(&mut e);
        front(&mut e);
        let mut plan = FrontlinePlan::new(FrontlineId(0), ATT);
        plan.stance = FrontlineStance::Defend;
        e.frontlines.plans.insert((FrontlineId(0), ATT), plan);
        assert_eq!(m1_after_the_period(&mut e).len(), 1);
        // SufficientAdvantage(攻撃側・防御側・支持国)。
        for c in [ATT, DEF, ATT_SUPPORT] {
            let mut e = env();
            arm_both(&mut e);
            observe(&mut e, c, MilitaryAiDecisionReason::SufficientAdvantage);
            assert!(m1_after_the_period(&mut e).is_empty(), "SufficientAdvantage {c:?}");
        }
    }

    #[test]
    fn m1_treats_supply_shortage_with_a_favourable_ratio_as_an_attack_intent() {
        // 補給が回復すれば攻勢に移る陣営(戦力比が開始条件 1.10 以上)は「意思あり」。
        let mut e = env();
        arm_both(&mut e);
        observe(&mut e, ATT, MilitaryAiDecisionReason::SupplyShortage);
        {
            let s = e.ai.ai_states.get_mut(&ATT).unwrap();
            s.estimated_own_power = 22_000;
            s.estimated_enemy_power = 20_000;
        }
        assert!(m1_after_the_period(&mut e).is_empty(), "比 1.10 ちょうど");
        // 開始条件に届かない戦力比なら、補給不足は攻勢の意思ではない。
        let mut e = env();
        arm_both(&mut e);
        observe(&mut e, ATT, MilitaryAiDecisionReason::SupplyShortage);
        {
            let s = e.ai.ai_states.get_mut(&ATT).unwrap();
            s.estimated_own_power = 21_999;
            s.estimated_enemy_power = 20_000;
        }
        assert_eq!(m1_after_the_period(&mut e).len(), 1);
    }

    #[test]
    fn m1_requires_both_sides_including_supporters_to_have_divisions() {
        // 片側だけが武装 → M1 ではなく E1 の分岐(365 日で終結、種別 E1)。
        let mut e = env();
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        run_kind(&mut e, START, None);
        assert_eq!(
            run_kind(&mut e, "1801/01/15", None),
            vec![(WID, StalemateKind::OneSidePowerless)]
        );
        // 支持国だけが武装している陣営も「武装」と数え、両陣営が武装なら M1。
        let mut e = env();
        e.military.add_division(division(0, ATT_SUPPORT, DivisionStatus::Idle));
        e.military.add_division(division(1, DEF, DivisionStatus::Idle));
        observe(&mut e, ATT_SUPPORT, MilitaryAiDecisionReason::ComparableStrength);
        observe(&mut e, DEF, MilitaryAiDecisionReason::ComparableStrength);
        assert_eq!(m1_after_the_period(&mut e), vec![(WID, StalemateKind::MutualArmed)]);
        // 防御側の支持国だけが武装 + 攻撃側が武装。
        let mut e = env();
        e.wars.wars.get_mut(&WID).unwrap().defenders.insert(CountryId(3));
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        e.military.add_division(division(1, CountryId(3), DivisionStatus::Idle));
        observe(&mut e, ATT, MilitaryAiDecisionReason::ComparableStrength);
        observe(&mut e, CountryId(3), MilitaryAiDecisionReason::EnemyStronger);
        assert_eq!(m1_after_the_period(&mut e).len(), 1);
        // 両陣営とも師団なしは E1(M1 ではない)。
        let mut e = env();
        run_kind(&mut e, START, None);
        assert_eq!(
            run_kind(&mut e, "1801/01/15", None),
            vec![(WID, StalemateKind::OneSidePowerless)]
        );
    }

    #[test]
    fn m1_does_not_treat_a_missing_or_unevaluated_military_ai_decision_as_no_intent() {
        // 軍事 AI の記録そのものがない(旧 Save・ロード直後)。
        let mut e = env();
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        e.military.add_division(division(1, DEF, DivisionStatus::Idle));
        assert!(m1_after_the_period(&mut e).is_empty(), "記録なし");
        // 片方だけ未観測。
        let mut e = env();
        arm_both(&mut e);
        e.ai.ai_states.remove(&DEF);
        assert!(m1_after_the_period(&mut e).is_empty(), "防御側が未観測");
        // 既定値(NoActiveWar = まだ一度も評価されていない)。
        let mut e = env();
        arm_both(&mut e);
        observe(&mut e, ATT, MilitaryAiDecisionReason::NoActiveWar);
        assert!(m1_after_the_period(&mut e).is_empty(), "NoActiveWar は未評価");
        // 師団を持つ支持国が未観測。
        let mut e = env();
        arm_both(&mut e);
        e.military.add_division(division(2, ATT_SUPPORT, DivisionStatus::Idle));
        assert!(m1_after_the_period(&mut e).is_empty(), "武装した支持国が未観測");
        // 師団を持たない支持国は、記録がなくても攻勢できないので判断を妨げない。
        let mut e = env();
        arm_both(&mut e);
        assert!(!e.ai.ai_states.contains_key(&ATT_SUPPORT));
        assert_eq!(m1_after_the_period(&mut e).len(), 1);
        // 軍事 AI レジストリ自体が渡されない場合も成立しない。
        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
        let none = update_stalemate_records_with_kind(
            &mut e.wars,
            &e.states,
            &e.military,
            &e.battles,
            &e.frontlines,
            &e.countries,
            None,
            None,
            DAY_730,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn m1_is_excluded_by_an_occupation_or_a_war_goal_achievement_but_not_by_a_mere_claim() {
        // 通常の領土要求(戦争目標があるだけ)は係争ではない: 基本ケースは成立する(上のテスト)。
        // 占領(所有国≠支配国で、両者がこの戦争の参加国): 防御側による攻撃側の州の占領。
        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
        e.states.get_mut(StateId(0)).unwrap().controller_country = Some(DEF);
        assert!(run_kind(&mut e, "1800/02/15", None).is_empty());
        assert!(run_kind(&mut e, "1802/02/15", None).is_empty(), "防御側による占領");
        // 戦争目標の州(StateId(1))を攻撃側が支配している。
        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
        e.states.get_mut(StateId(1)).unwrap().controller_country = Some(ATT);
        assert!(run_kind(&mut e, "1800/02/15", None).is_empty());
        assert!(run_kind(&mut e, "1802/02/15", None).is_empty(), "戦争目標の達成");
        // war.occupied_states が空でない。
        let mut e = env();
        arm_both(&mut e);
        e.wars.wars.get_mut(&WID).unwrap().occupied_states.insert(StateId(1));
        run_kind(&mut e, START, None);
        assert!(run_kind(&mut e, DAY_730, None).is_empty());
        // 戦争の参加国でない国の州を参加国が支配している(この戦争の占領ではない)場合、
        // 起点はその支配変更でリセットされるが、期間満了後は成立する。
        let mut e = env();
        arm_both(&mut e);
        e.states = StateRegistry::build(vec![
            StateData { id: StateId(0), owner_country_id: ATT, ..Default::default() },
            StateData { id: StateId(1), owner_country_id: DEF, ..Default::default() },
            StateData { id: StateId(2), owner_country_id: CountryId(9), ..Default::default() },
        ]);
        run_kind(&mut e, START, None);
        e.states.get_mut(StateId(2)).unwrap().controller_country = Some(ATT);
        assert!(run_kind(&mut e, "1800/03/15", None).is_empty(), "進展としてリセット");
        assert_eq!(
            run_kind(&mut e, "1802/03/15", None),
            vec![(WID, StalemateKind::MutualArmed)],
            "参加国でない国の領土の支配は、この戦争の占領ではない"
        );
    }

    #[test]
    fn m1_requires_the_existing_white_peace_acceptance_in_both_directions() {
        // 既存の受諾判定(戦勝点 -10 以上)を通らない戦争は成立しない。
        let mut e = env();
        arm_both(&mut e);
        e.wars.wars.get_mut(&WID).unwrap().war_score = -50.0;
        assert!(m1_after_the_period(&mut e).is_empty());
        // 双方の受諾の合成: 片方向でも失敗すれば不成立。
        assert!(agreed_by_both(ATT, DEF, |_, _| true));
        assert!(!agreed_by_both(ATT, DEF, |p, _| p != ATT), "攻撃側→防御側が失敗");
        assert!(!agreed_by_both(ATT, DEF, |p, _| p != DEF), "防御側→攻撃側が失敗");
    }

    #[test]
    fn m1_excludes_player_wars_and_wars_without_goals() {
        for player in [ATT, DEF, ATT_SUPPORT] {
            let mut e = env();
            arm_both(&mut e);
            run_kind(&mut e, START, Some(player));
            assert!(e.wars.stalemate_records.is_empty(), "{player:?}");
            assert!(run_kind(&mut e, DAY_730, Some(player)).is_empty());
        }
        let mut e = env();
        arm_both(&mut e);
        e.wars.wars.get_mut(&WID).unwrap().war_goals.clear();
        assert!(m1_after_the_period(&mut e).is_empty());
    }

    #[test]
    fn m1_allows_settlement_right_after_an_attack_intent_has_disappeared() {
        // P31-002 の仕様: 実際の進展が 730 日なく、現在の全条件を満たすなら、攻勢の意思が消えた
        // 直後でも許可する(猶予状態は持たない)。
        let mut e = env();
        arm_both(&mut e);
        run_kind(&mut e, START, None);
        observe(&mut e, ATT, MilitaryAiDecisionReason::SufficientAdvantage);
        assert!(run_kind(&mut e, DAY_729, None).is_empty());
        assert!(run_kind(&mut e, DAY_730, None).is_empty(), "意思がある日は成立しない");
        observe(&mut e, ATT, MilitaryAiDecisionReason::ComparableStrength);
        assert_eq!(
            run_kind(&mut e, "1802/01/16", None),
            vec![(WID, StalemateKind::MutualArmed)],
            "意思が消えた翌日に成立する"
        );
    }

    #[test]
    fn m1_returns_only_the_stalled_war_and_other_wars_are_untouched() {
        let mut e = env();
        arm_both(&mut e);
        let other = WarId(8);
        let mut w = war(other, "1800/01/01");
        w.attackers = [CountryId(5)].into_iter().collect();
        w.defenders = [CountryId(6)].into_iter().collect();
        w.primary_attacker = Some(CountryId(5));
        w.primary_defender = Some(CountryId(6));
        e.wars.wars.insert(other, w);
        e.military.add_division(division(2, CountryId(5), DivisionStatus::Idle));
        e.military.add_division(division(3, CountryId(6), DivisionStatus::Moving));
        observe(&mut e, CountryId(5), MilitaryAiDecisionReason::ComparableStrength);
        observe(&mut e, CountryId(6), MilitaryAiDecisionReason::ComparableStrength);
        assert_eq!(m1_after_the_period(&mut e), vec![(WID, StalemateKind::MutualArmed)]);
        // 終結した戦争の記録は削除され、他の戦争の記録は残る。
        e.wars.wars.get_mut(&WID).unwrap().status = WarStatus::WhitePeace;
        run_kind(&mut e, "1802/01/16", None);
        assert!(!e.wars.stalemate_records.contains_key(&WID));
        assert!(e.wars.stalemate_records.contains_key(&other));
        // 終結済みの戦争は再評価されない(二重終結しない)。
        assert!(run_kind(&mut e, "1804/01/16", None).iter().all(|(id, _)| *id != WID));
    }

    #[test]
    fn e1_still_settles_a_one_sided_war_at_365_days_with_its_own_reason() {
        let mut e = env();
        e.military.add_division(division(0, ATT, DivisionStatus::Idle));
        run_kind(&mut e, START, None);
        assert!(run_kind(&mut e, "1801/01/14", None).is_empty());
        assert_eq!(
            run_kind(&mut e, "1801/01/15", None),
            vec![(WID, StalemateKind::OneSidePowerless)]
        );
    }
}
