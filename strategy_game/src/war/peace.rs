use crate::app::time::GameDate;
use crate::common::{ClaimId, CountryId, DivisionId, StateId, WarId};
use crate::diplomacy::claims::ClaimRegistry;
use crate::diplomacy::crisis::CrisisRegistry;
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::battle::{BattleRegistry, BattleStatus};
use crate::military::data::{DivisionStatus, MilitaryRegistry};
use crate::state::data::StateRegistry;
use crate::war::data::{War, WarRegistry, WarStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 講和条件の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeaceTerm {
    /// 白紙講和
    WhitePeace,
    /// 戦争目標地域の割譲
    CedeWarGoalRegion,
    /// 攻撃側の敗北受諾
    AttackerConcedes,
    /// P21-021: 攻撃側の代表国(primary_attacker)が国家消滅により消滅した。
    /// 所有権移転は一切発生しない(消滅時に既に完了済み)——防御側の勝利として
    /// 記録するためだけの終結理由。`country::elimination::apply_country_
    /// elimination`の他War清掃からのみ使われ、プレイヤーの講和提案
    /// (`can_accept_peace_offer`)からは選択できない。
    AttackerEliminated,
    /// P21-021: 防御側の代表国(primary_defender)が国家消滅により消滅した。
    /// `AttackerEliminated`と対称(攻撃側の勝利として記録)。
    DefenderEliminated,
}

impl PeaceTerm {
    /// 表示用の翻訳キー(P20-009)。UI側で`localization::t()`により言語ごとの表示名へ解決する。
    /// `War.applied_terms`にもこのキーがそのまま(未翻訳で)格納され、表示直前に解決される。
    pub fn display_name(self) -> &'static str {
        match self {
            PeaceTerm::WhitePeace => "peace_term.white_peace",
            PeaceTerm::CedeWarGoalRegion => "peace_term.cede_war_goal_region",
            PeaceTerm::AttackerConcedes => "peace_term.attacker_concedes",
            PeaceTerm::AttackerEliminated => "peace_term.attacker_eliminated",
            PeaceTerm::DefenderEliminated => "peace_term.defender_eliminated",
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
    country_registry: Option<&crate::country::CountryRegistry>,
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

    // P21-021: 消滅済み国家は講和提案の提案者にも受領者にもなれない
    // (通常はapply_country_eliminationが全Warの参加国集合から即座に除去するため
    // 構造的にも起こり得ないが、Domain APIレベルの防御としてここでも明示的に検証する)。
    if let Some(c_reg) = country_registry {
        if c_reg
            .get(offer.proposer_country_id)
            .is_none_or(|c| !c.is_active())
        {
            return Err("war_error.peace.proposer_missing");
        }
        if c_reg
            .get(offer.recipient_country_id)
            .is_none_or(|c| !c.is_active())
        {
            return Err("war_error.peace.recipient_missing");
        }
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
        // P21-021: `AttackerEliminated`/`DefenderEliminated`は
        // `country::elimination::apply_country_elimination`が内部的にのみ使う
        // 終結理由であり、プレイヤー/AIの講和提案(`PeaceOffer`)としては
        // 一切成立し得ない(講和ボタンUIはこれらのvariantを選択肢として構築しない)。
        PeaceTerm::AttackerEliminated | PeaceTerm::DefenderEliminated => {
            return Err("war_error.peace.not_a_player_offerable_term");
        }
    }

    Ok(())
}

/// `excluded_war_id`以外のActive Warで、`a`と`b`が敵対する陣営に分かれているか。
/// (`WarRegistry::are_countries_at_war`と同じ判定だが、終結処理中のWar自身を除く。
/// 講和ステップ2・5の時点では終結するWarの`status`はまだ`Active`のため、除外が必須。)
fn are_enemies_in_other_active_war(
    war_registry: &WarRegistry,
    excluded_war_id: WarId,
    a: CountryId,
    b: CountryId,
) -> bool {
    a != b
        && war_registry.wars.values().any(|w| {
            w.id != excluded_war_id
                && w.status == WarStatus::Active
                && ((w.attackers.contains(&a) && w.defenders.contains(&b))
                    || (w.attackers.contains(&b) && w.defenders.contains(&a)))
        })
}

/// P28-008: 終結するWarに帰属する占領だけを、所有国の支配へ戻す。
///
/// 現行の占領処理(`military::invasion::occupy_state`)は`controller_country`しか
/// 書かず、`state.war_id`は本番で設定されない(`war::occupation::process_occupation`は
/// 本番Systemから呼ばれない)。そのため占領元Warは州データから直接は分からず、
/// 次の条件で帰属を判定する(`war_id`には一切依存しない)。
///
/// 州の`owner`・`controller`が異なる(=占領中)とき、次をすべて満たす占領だけを戻す。
/// - `owner`・`controller`がともに終結Warの全参加国(`attackers ∪ defenders`。
///   主要当事国だけでなく支持国も含む)に属する
/// - 他のActive Warで`owner`と`controller`が敵対陣営に分かれていない
///   (そのWarが続く限り、この占領は別Warによって維持されている)
///
/// 真理値表(W=終結するWar):
///
/// | 状況 | owner∈P(W) | controller∈P(W) | 他Active Warで敵対 | 結果 |
/// |---|---|---|---|---|
/// | 単独War: WでXがY所有州を占領 | ○ | ○ | × | 戻す |
/// | 同じ防御国Yへの別War B(Z対Y)でZが占領した州 | ○ | × | - | 保持 |
/// | 当事国共有: 別War(V対X)でVが占領したX所有州 | ○ | × | - | 保持 |
/// | 支持国Q所有州をXが占領(QはWの防御側参加国) | ○ | ○ | × | 戻す |
/// | WでもXがYの州を占領、かつ別Active WarでもX・Yが敵対 | ○ | ○ | ○ | 保持 |
/// | 所有国がWの参加国でない州 | × | - | - | 保持 |
///
/// 割譲された州は呼び出し前に`controller==owner`になっているため対象外。
fn normalize_occupations_of_ended_war(
    war_id: WarId,
    state_registry: &mut StateRegistry,
    war_registry: &WarRegistry,
) {
    let participants: HashSet<CountryId> = match war_registry.wars.get(&war_id) {
        Some(w) => w
            .attackers
            .iter()
            .chain(w.defenders.iter())
            .copied()
            .collect(),
        None => return,
    };

    for state in state_registry.states.iter_mut() {
        if state.is_sea {
            continue;
        }
        let owner = state.owner_country_id;
        let controller = state.controller();
        let belongs_to_ended_war = controller != owner
            && participants.contains(&owner)
            && participants.contains(&controller)
            && !are_enemies_in_other_active_war(war_registry, war_id, owner, controller);
        if belongs_to_ended_war {
            state.controller_country = Some(owner);
            state.occupation_progress = 0.0;
            state.original_owner = None;
            state.war_id = None;
        } else if state.war_id == Some(war_id) {
            // 終結済みWarを指す参照を残さない(本番では設定されない旧フィールドの衛生処理)。
            state.war_id = None;
        }
    }
}

/// P28-008: 終結するWar以外のActive Warで、所有国が州のcontrollerまたはownerと敵対している
/// Division(=別Warで正当に敵領内にいる)のIDを返す。講和の帰還処理の対象外とする。
fn divisions_legitimately_in_enemy_territory(
    war_id: WarId,
    state_registry: &StateRegistry,
    military_registry: &MilitaryRegistry,
    war_registry: &WarRegistry,
) -> HashSet<DivisionId> {
    military_registry
        .divisions
        .values()
        .filter(|d| d.status != DivisionStatus::Destroyed && d.manpower > 0)
        .filter(|d| {
            state_registry.get(d.current_state).is_some_and(|s| {
                are_enemies_in_other_active_war(war_registry, war_id, d.owner, s.controller())
                    || are_enemies_in_other_active_war(
                        war_registry,
                        war_id,
                        d.owner,
                        s.owner_country_id,
                    )
            })
        })
        .map(|d| d.id)
        .collect()
}

/// 戦争終結後の敵国領残存陸軍の自国領域への安全帰還処理
pub fn relocate_hostile_territory_divisions(
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
) {
    relocate_hostile_territory_divisions_except(state_registry, military_registry, &HashSet::new());
}

/// `relocate_hostile_territory_divisions`の本体。`skip`に含まれるDivisionは移動しない
/// (P28-008: 別のActive Warで正当に敵領内にいるDivision)。
pub fn relocate_hostile_territory_divisions_except(
    state_registry: &StateRegistry,
    military_registry: &mut MilitaryRegistry,
    skip: &HashSet<DivisionId>,
) {
    for division in military_registry.divisions.values_mut() {
        if division.status == DivisionStatus::Destroyed || division.manpower == 0 {
            continue;
        }
        if skip.contains(&division.id) {
            continue;
        }

        let curr_state = match state_registry.get(division.current_state) {
            Some(s) => s,
            None => continue,
        };

        // 自国所有または自国支配の地域に居る場合は移動不要
        if curr_state.owner_country_id == division.owner
            || curr_state.controller() == division.owner
        {
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
                        && (ns.controller() == division.owner
                            || ns.owner_country_id == division.owner)
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
                !s.is_sea
                    && (s.controller() == division.owner || s.owner_country_id == division.owner)
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

/// P21-019: `War.source_crisis_id`が示すCrisis由来lineageを検証する純粋関数。
/// Registryへの書き込みは一切行わない(検証のみ)。
///
/// `war.source_crisis_id`が`None`(通常War)の場合は`Ok(None)`を返し、
/// Claim消費対象が無いことを示す。`Some`の場合は以下をすべて検証し、
/// 一つでも不整合があれば最初に見つかった時点で`Err`を返す(§5要求):
///
/// 1. `source_crisis_id`のCrisisが存在する
/// 2. CrisisのinitiatorがWarのprimary attackerと一致する
/// 3. CrisisのtargetがWarのprimary defenderと一致する
/// 4. Crisisが参照するClaimが存在する(`related_claim_id`)
/// 5. Claimのclaimant国がCrisis initiatorと一致する
/// 6. Claimのtarget_stateがCrisis自身のwar_goal target_stateと一致する
/// 7. そのtarget_stateがState Registryに存在する
///
/// P21-020: 「対象州の現在の法的所有国がまだCrisis targetのまま」という検証は
/// ここでは行わない——`execute_peace_settlement`が`CedeWarGoalRegion`の場合に
/// 呼ぶ共通Domain API `state::ownership::preflight_transfer_state_ownership`の
/// `expected_current_owner`検証が、Crisis由来War・通常War問わず同じ条件を
/// 一箇所だけで保証する(以前はCrisis由来Warだけがここで個別に検証しており、
/// 通常Warには同じ保護が無い非対称があった。§5:同じvalidationを複数箇所へ
/// 複製しない)。
///
/// `War.primary_attacker_id()`/`primary_defender_id()`はattackers/defendersが
/// 空だとpanicするため、ここでは呼ばない(Save検証と異なりこの経路は
/// `execute_peace_settlement`からのみ呼ばれ、`declare_war`が必ず
/// initiator/targetを挿入してから`War`を構築するため実際には空になり得ないが、
/// 呼び出し規約を安全側に統一するため`war_registry`側の呼び出し元で
/// 計算済みの値をそのまま受け取る)。
pub(crate) fn resolve_crisis_war_lineage(
    war: &War,
    attacker: CountryId,
    defender: CountryId,
    crisis_registry: &CrisisRegistry,
    claim_registry: &ClaimRegistry,
    state_registry: &StateRegistry,
) -> Result<Option<ClaimId>, &'static str> {
    let Some(source_crisis_id) = war.source_crisis_id else {
        return Ok(None);
    };

    let crisis = crisis_registry
        .crises
        .get(&source_crisis_id)
        .ok_or("war_error.peace.crisis_lineage_missing_crisis")?;

    if crisis.initiator != attacker {
        return Err("war_error.peace.crisis_lineage_attacker_mismatch");
    }
    if crisis.target != defender {
        return Err("war_error.peace.crisis_lineage_defender_mismatch");
    }

    let claim_id = crisis
        .related_claim_id
        .ok_or("war_error.peace.crisis_lineage_missing_claim_ref")?;
    let claim = claim_registry
        .claims
        .get(&claim_id)
        .ok_or("war_error.peace.crisis_lineage_missing_claim")?;

    if claim.claimant_country != crisis.initiator {
        return Err("war_error.peace.crisis_lineage_claimant_mismatch");
    }

    let crisis_target_state = crisis
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied())
        .ok_or("war_error.peace.crisis_lineage_missing_target_state")?;

    if claim.target_state != crisis_target_state {
        return Err("war_error.peace.crisis_lineage_target_state_mismatch");
    }
    if state_registry.get(crisis_target_state).is_none() {
        return Err("war_error.peace.crisis_lineage_state_missing");
    }

    Ok(Some(claim_id))
}

/// P21-020(ユーザー指摘対応): 州所有権が実際に移転した直後、その州を目標とする
/// 他のActive War(`excluding_war_id`自身を除く)を全て白紙講和(`WhitePeace`)で
/// 安全に失効させる。
///
/// **背景**: `can_declare_war_with_date`は対象州のグローバルな一意性を検証
/// しないため(§実装前調査17)、異なる攻撃国が同じ防御国の同じ州を目標にする
/// 複数のActive Warが正当に同時存在し得る。1件目の講和で州の所有権が実際に
/// 移った瞬間、残りのWarは「対象州をdefenderから割譲させる」という目標を
/// 構造的に永久達成不能になる——放置すると、自動降伏講和が毎日同じ割譲を
/// 試みては`state_error.ownership.current_owner_mismatch`で拒否され続けて
/// Warが永久停止し、Save validationも(Active WarのWar goal所有者検証により)
/// 壊れたSaveとして拒否し続ける。
///
/// **白紙講和を選ぶ理由**: `WhitePeace`は所有権移転・Claim消費のいずれも行わない
/// (`execute_peace_settlement`のWhitePeace分岐は`ownership_plan`を計算しない)
/// ため、この関数から`execute_peace_settlement`を再帰的に呼んでも無限再帰・
/// 二重の所有権移転が発生しない。対象Warの`related_claim_id`由来のClaimは
/// `Active`のまま残る(達成できなかっただけで、請求自体が無効になったわけでは
/// ないため、消費しない)。
///
/// WarId昇順で決定論的に処理する。
#[allow(clippy::too_many_arguments)]
fn invalidate_other_wars_targeting_state(
    state_id: StateId,
    excluding_war_id: WarId,
    current_date_str: &str,
    state_registry: &mut StateRegistry,
    war_registry: &mut WarRegistry,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    frontline_registry: &mut crate::war::frontline::FrontlineRegistry,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    country_registry: &mut crate::country::CountryRegistry,
    army_registry: &mut crate::military::army::ArmyRegistry,
    justification_registry: &mut crate::war::justification::WarJustificationRegistry,
    power_registry: &mut crate::country::power::CountryPowerRegistry,
) {
    let mut stale_war_ids: Vec<WarId> = war_registry
        .wars
        .values()
        .filter(|w| {
            w.id != excluding_war_id
                && w.status == WarStatus::Active
                && w.war_goals
                    .iter()
                    .any(|g| g.target_states.contains(&state_id))
        })
        .map(|w| w.id)
        .collect();
    stale_war_ids.sort_by_key(|w| w.0);

    for stale_war_id in stale_war_ids {
        let _ = execute_peace_settlement(
            stale_war_id,
            PeaceTerm::WhitePeace,
            "War Goal No Longer Achievable",
            current_date_str,
            state_registry,
            war_registry,
            military_registry,
            battle_registry,
            diplomacy_registry,
            frontline_registry,
            crisis_registry,
            claim_registry,
            country_registry,
            army_registry,
            justification_registry,
            power_registry,
        );
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
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    country_registry: &mut crate::country::CountryRegistry,
    army_registry: &mut crate::military::army::ArmyRegistry,
    justification_registry: &mut crate::war::justification::WarJustificationRegistry,
    power_registry: &mut crate::country::power::CountryPowerRegistry,
) -> Result<(), &'static str> {
    let war = war_registry
        .wars
        .get_mut(&war_id)
        .ok_or("war_error.peace.war_not_found")?;

    // 二重適用防止
    if war.status != WarStatus::Active {
        return Ok(());
    }

    // P21-016(二次レビュー対応): 多国間参加者がいても、講和条約の当事国(領土移転先・
    // winner・休戦相手)は`HashSet`走査順に依存しない明示的な代表国を使う
    // (以前はここだけ`.iter().next()`の取り残しがあった)。
    let attacker = war.primary_attacker_id();
    let defender = war.primary_defender_id();

    let target_state_id = war
        .war_goals
        .first()
        .and_then(|g| g.target_states.first().copied());

    // P21-019: Crisis由来War(`source_crisis_id.is_some()`)のlineageを、実際の
    // 変更を一切行う前に検証する。通常War(`None`)は`Ok(None)`で即座に素通りする。
    // 不整合が見つかった場合はここで即座にエラーを返し、以降の領土移転・占領解除・
    // 講和状態更新を含む一切の変更を行わない(§5: 全か無かの原子性)。
    let claim_to_consume = resolve_crisis_war_lineage(
        war,
        attacker,
        defender,
        crisis_registry,
        claim_registry,
        state_registry,
    )?;

    // P21-020: `CedeWarGoalRegion`の州所有権移転を、共通Domain API
    // `state::ownership::preflight_transfer_state_ownership`で事前検証する。
    // `expected_current_owner = defender`——Crisis由来War・通常War問わず、
    // 「対象州の現在の法的所有国がdefenderのまま」であることと首都再配置の
    // 両方をここ一箇所で保証する(以前はCrisis由来Warにしか所有者検証が
    // 無かった非対称・首都再配置の重複実装を解消)。実際の変更を一切行う前に
    // 検証するため、失敗時はここでErrを返して他のステップへ進まない。
    //
    // P21-021: `LastOwnedLandState`(=defenderがこの州を失うと陸州0になる)が
    // 返った場合だけ、`country::elimination`の国家消滅Domain APIへフォールバック
    // する。首都再配置を試みない専用preflight(`preflight_final_state_transfer_
    // for_elimination`)で同じ移転を検証し直す(defenderに新しい首都という概念は
    // 存在しないため)。
    enum CessionPlan {
        Normal(crate::state::ownership::ValidatedOwnershipTransfer),
        Elimination(crate::country::elimination::CountryEliminationPlan),
    }
    let cession_plan = match (term, target_state_id) {
        (PeaceTerm::CedeWarGoalRegion, Some(target_id)) => {
            match crate::state::ownership::preflight_transfer_state_ownership(
                target_id,
                defender,
                attacker,
                state_registry,
                country_registry,
            ) {
                Ok(plan) => Some(CessionPlan::Normal(plan)),
                Err(crate::state::ownership::OwnershipTransferError::LastOwnedLandState) => {
                    let elim_plan = crate::country::elimination::preflight_country_elimination(
                        target_id,
                        defender,
                        attacker,
                        current_date_str,
                        state_registry,
                        country_registry,
                    )
                    .map_err(|e| e.localization_key())?;
                    Some(CessionPlan::Elimination(elim_plan))
                }
                Err(e) => return Err(e.localization_key()),
            }
        }
        _ => None,
    };

    // 1. 講和条件に応じた領土移転と戦争ステータスの更新
    let mut transferred_state: Option<StateId> = None;
    let (final_status, winner) = match term {
        PeaceTerm::CedeWarGoalRegion => {
            match cession_plan {
                Some(CessionPlan::Normal(plan)) => {
                    crate::state::ownership::apply_ownership_transfer(
                        plan,
                        state_registry,
                        country_registry,
                    )
                    .map_err(|e| e.localization_key())?;
                    transferred_state = Some(plan.state_id());
                }
                Some(CessionPlan::Elimination(elim_plan)) => {
                    // P21-021: defenderの最後の所有陸州を移転すると同時に、
                    // 国家消滅の全清掃(軍事・他War・外交・Crisis・Claim・正当化)を
                    // 行う。causing warである`war_id`自身は清掃ループから除外され、
                    // 以降のこの関数自身のステップ(占領解除・戦闘中止・前線削除・
                    // 休戦作成・終了情報更新・Claim消費)で通常どおり終結する。
                    let outcome = crate::country::elimination::apply_country_elimination(
                        elim_plan,
                        Some(war_id),
                        None,
                        current_date_str,
                        state_registry,
                        country_registry,
                        military_registry,
                        army_registry,
                        frontline_registry,
                        war_registry,
                        battle_registry,
                        diplomacy_registry,
                        crisis_registry,
                        claim_registry,
                        justification_registry,
                        power_registry,
                    )
                    .map_err(|e| e.localization_key())?;
                    transferred_state = Some(outcome.final_state_id);
                }
                None => {}
            }
            (WarStatus::AttackerVictory, Some(attacker))
        }
        PeaceTerm::AttackerConcedes => (WarStatus::DefenderVictory, Some(defender)),
        PeaceTerm::WhitePeace => (WarStatus::WhitePeace, None),
        // P21-021: 消滅した側の対陣営を勝者として記録する(白紙講和にしない)。
        // 所有権移転は発生しない——消滅処理自体が既に完了しているため。
        PeaceTerm::AttackerEliminated => (WarStatus::DefenderVictory, Some(defender)),
        PeaceTerm::DefenderEliminated => (WarStatus::AttackerVictory, Some(attacker)),
    };

    // P21-020(ユーザー指摘対応): 州所有権が実際に移転した場合、その州を目標と
    // する他のActive War(このWar自身を除く)は構造的に永久達成不能になった
    // ため、白紙講和で安全に失効させる(詳細は`invalidate_other_wars_targeting_
    // state`のドキュメント参照)。
    if let Some(state_id) = transferred_state {
        invalidate_other_wars_targeting_state(
            state_id,
            war_id,
            current_date_str,
            state_registry,
            war_registry,
            military_registry,
            battle_registry,
            diplomacy_registry,
            frontline_registry,
            crisis_registry,
            claim_registry,
            country_registry,
            army_registry,
            justification_registry,
            power_registry,
        );
    }

    // 2. 占領状態の正常化（このWarに帰属する占領だけを所有国へ戻す。P28-008）
    normalize_occupations_of_ended_war(war_id, state_registry, war_registry);

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

    // 5. 敵国領残存陸軍の帰還(別のActive Warで正当に敵領内にいるDivisionは対象外。P28-008)
    let legitimately_deployed = divisions_legitimately_in_enemy_territory(
        war_id,
        state_registry,
        military_registry,
        war_registry,
    );
    relocate_hostile_territory_divisions_except(
        state_registry,
        military_registry,
        &legitimately_deployed,
    );

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

    // 8. P21-019: Crisis由来Warなら、講和結果(白紙講和・攻撃側/防御側勝利いずれでも)
    // を問わず元Claimを消費済みへ移行する。`mark_consumed`は対象Claimが既に
    // Consumedでも`true`を返す冪等な操作であり(このWar自身は上の二重適用防止で
    // 既に一度しかここへ到達しないが)、念のため多重適用されても安全。
    if let Some(claim_id) = claim_to_consume {
        claim_registry.mark_consumed(claim_id);
    }

    Ok(())
}
