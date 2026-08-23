//! P21-021: 国家消滅(完全併合)の共通Domain API。
//!
//! 「最後の所有陸州」を移転してもらう国家を`CountryRegistry`から物理削除することは
//! 一切行わない――`CountryId`とその国家データはWar/Claim/Crisisの履歴参照として
//! 永続的に残す。代わりに`CountryData.status`を`CountryStatus::Eliminated`へ
//! 遷移させ、軍事・戦争・外交・Crisis・Claim・正当化の各Registryから、もう存在しない
//! 国家への参照を一括で整理する。
//!
//! `state::ownership`のpreflight/apply分離パターン(TOCTOU再検証込み)をそのまま
//! 踏襲する。呼び出し元(`war::peace::execute_peace_settlement`のCedeWarGoalRegion
//! 分岐、`diplomacy::crisis_response::accept_demand`)が、通常の
//! `state::ownership::preflight_transfer_state_ownership`が`LastOwnedLandState`を
//! 返した場合にのみこちらへフォールバックする。
//!
//! **消滅を引き起こした戦争(causing war)自身の終結処理は、この関数の対象外**――
//! 呼び出し元(`execute_peace_settlement`)が自身の既存フローで(end_date・
//! end_reason・truce・Claim消費を含め)それを終結させるため、二重処理を避ける
//! ためにこの関数は`causing_war_id`を明示的に除外する。消滅国家が参加する
//! **他の**Active Warは、この関数がここで直接処理する(§9: 主要参加国として
//! 参加していれば、対陣営の勝利として終結(`PeaceTerm::AttackerEliminated`/
//! `DefenderEliminated`、白紙講和ではない)、支持国としてのみ参加していれば
//! 陣営集合から除去するだけ)。

use crate::common::{ClaimId, CountryId, DiplomaticCrisisId, StateId, WarId};
use crate::country::power::CountryPowerRegistry;
use crate::country::{CountryRegistry, CountryStatus};
use crate::diplomacy::claims::{ClaimRegistry, ClaimStatus};
use crate::diplomacy::crisis::{CrisisPhase, CrisisRegistry, is_unresolved_phase};
use crate::diplomacy::data::DiplomacyRegistry;
use crate::military::army::ArmyRegistry;
use crate::military::battle::BattleRegistry;
use crate::military::data::MilitaryRegistry;
use crate::state::data::StateRegistry;
use crate::state::ownership::{self, OwnershipTransferError};
use crate::war::data::{WarRegistry, WarStatus};
use crate::war::frontline::FrontlineRegistry;
use crate::war::justification::WarJustificationRegistry;

/// 国家消滅の失敗理由。州移転自体の失敗は`state::ownership`のエラーをそのまま
/// 包む(重複した変種を持たない――`OwnershipTransferError::localization_key()`を
/// そのまま委譲する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountryEliminationError {
    StateTransfer(OwnershipTransferError),
    /// 対象国家が既に`Eliminated`(重複消滅は許可しない)。
    AlreadyEliminated,
    /// 併合先国家が存在しない、または既に`Eliminated`。
    AnnexerNotActive,
    /// P21-021(§6アトミック性): 消滅国家が主要参加国である、causing war以外の
    /// 他Warのうち少なくとも1件が、Crisis由来lineage不整合(壊れた
    /// `source_crisis_id`参照等)により終結できない。この検証は一切の変更を
    /// 加える前(`apply_country_elimination`の最初のステップ)で行うため、
    /// このエラーが返った時点でどのRegistryも変更されていないことを保証する
    /// (全か無かの原子性)。内包する`&'static str`は`war_error.peace.*`系の
    /// 翻訳キー(`resolve_crisis_war_lineage`が返すものをそのまま透過する)。
    OtherWarSettlementPreflight(&'static str),
}

impl CountryEliminationError {
    /// 表示用の翻訳キー(P20-009の規約)。UI側で`localization::t()`により言語ごとの
    /// 表示名へ解決する。
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::StateTransfer(e) => e.localization_key(),
            Self::AlreadyEliminated => "country_error.elimination.already_eliminated",
            Self::AnnexerNotActive => "country_error.elimination.annexer_not_active",
            Self::OtherWarSettlementPreflight(key) => key,
        }
    }
}

/// 検証済みの国家消滅計画(Registryを一切変更せずに構築される)。
/// `preflight_country_elimination`でのみ構築でき、`apply_country_elimination`
/// でのみ消費できる。
#[derive(Debug, Clone, PartialEq)]
pub struct CountryEliminationPlan {
    state_transfer: ownership::ValidatedOwnershipTransfer,
    eliminated_country: CountryId,
    annexed_by: CountryId,
    eliminated_date: String,
}

/// 国家消滅の適用結果。UI通知・完了報告・テスト検証のために、実際に変更された
/// 内容を全て明示的に返す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryEliminationOutcome {
    pub eliminated_country: CountryId,
    pub annexed_by: CountryId,
    pub final_state_id: StateId,
    pub eliminated_date: String,
    pub divisions_disbanded: usize,
    /// 消滅国家が主要参加国だった、causing war以外のActive War
    /// (対陣営の勝利として終結、白紙講和ではない)。
    pub other_wars_ended: Vec<WarId>,
    /// 消滅国家が支持国としてのみ参加していたActive War(陣営集合から除去のみ)。
    pub wars_supporter_removed: Vec<WarId>,
    pub crises_cancelled: Vec<DiplomaticCrisisId>,
    pub claims_invalidated: Vec<ClaimId>,
    pub justifications_removed: Vec<usize>,
}

/// 国家消滅の事前検証(Registryを一切変更しない純粋関数)。
///
/// 検証順序: 消滅対象国家が存在し`Active`であること → 併合先国家が存在し
/// `Active`であること → 最後の所有陸州の移転自体が`state::ownership`の
/// 通常validation(State存在・非海州・現在所有者一致・新旧所有者相違)を満たすこと。
/// 首都再配置は試みない(消滅国家に「新しい首都」という概念は存在しない)。
pub fn preflight_country_elimination(
    final_state_id: StateId,
    eliminated_country: CountryId,
    annexed_by: CountryId,
    eliminated_date: &str,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
) -> Result<CountryEliminationPlan, CountryEliminationError> {
    let eliminated =
        country_registry
            .get(eliminated_country)
            .ok_or(CountryEliminationError::StateTransfer(
                OwnershipTransferError::CountryNotFound,
            ))?;
    if eliminated.is_eliminated() {
        return Err(CountryEliminationError::AlreadyEliminated);
    }

    let annexer =
        country_registry
            .get(annexed_by)
            .ok_or(CountryEliminationError::StateTransfer(
                OwnershipTransferError::CountryNotFound,
            ))?;
    if !annexer.is_active() {
        return Err(CountryEliminationError::AnnexerNotActive);
    }

    let state_transfer = ownership::preflight_final_state_transfer_for_elimination(
        final_state_id,
        eliminated_country,
        annexed_by,
        state_registry,
        country_registry,
    )
    .map_err(CountryEliminationError::StateTransfer)?;

    Ok(CountryEliminationPlan {
        state_transfer,
        eliminated_country,
        annexed_by,
        eliminated_date: eliminated_date.to_string(),
    })
}

/// 消滅国家が主要参加国(`primary_attacker_id`/`primary_defender_id`)である
/// causing war以外のActive Warを、消滅国家の対陣営の勝利として終結させる
/// (§4ユーザー指摘対応: 白紙講和(引き分け)ではなく、明示的な
/// `PeaceTerm::AttackerEliminated`/`DefenderEliminated`を使う——消滅国家が
/// 攻撃側ならAttackerEliminated(防御側勝利)、防御側ならDefenderEliminated
/// (攻撃側勝利))。`war::peace::execute_peace_settlement`をそのまま再利用する
/// (占領解除・戦闘中止・前線削除・休戦作成・Crisis由来Claim消費の全てを
/// 重複実装しない)。いずれの新PeaceTermも所有権移転を一切計算しないため、
/// この関数からの呼び出しが新たな`LastOwnedLandState`を生む(=再帰的に
/// 国家消滅を誘発する)ことは構造的に起こり得ない。
///
/// この関数は`apply_country_elimination`が既にCrisis由来lineageを
/// 事前検証済みの`war_id`に対してのみ呼ばれるため、`execute_peace_settlement`
/// が失敗することはない(§6アトミック性: 失敗し得るステップは全て事前に
/// 検証済み)。
#[allow(clippy::too_many_arguments)]
fn end_other_primary_war_due_to_elimination(
    war_id: WarId,
    eliminated_country: CountryId,
    current_date_str: &str,
    state_registry: &mut StateRegistry,
    war_registry: &mut WarRegistry,
    military_registry: &mut MilitaryRegistry,
    battle_registry: &mut BattleRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    frontline_registry: &mut FrontlineRegistry,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    country_registry: &mut CountryRegistry,
    army_registry: &mut ArmyRegistry,
    justification_registry: &mut WarJustificationRegistry,
    power_registry: &mut CountryPowerRegistry,
) {
    let Some(war) = war_registry.wars.get(&war_id) else {
        return;
    };
    let term = if war.primary_attacker_id() == eliminated_country {
        crate::war::peace::PeaceTerm::AttackerEliminated
    } else {
        crate::war::peace::PeaceTerm::DefenderEliminated
    };

    let result = crate::war::peace::execute_peace_settlement(
        war_id,
        term,
        "Participant Country Eliminated",
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
    debug_assert!(
        result.is_ok(),
        "execute_peace_settlement must not fail here: apply_country_elimination already \
         preflighted this war's Crisis/Claim lineage before making any change (§6 atomicity)"
    );
}

/// 検証済み国家消滅計画を適用する。適用直前にTOCTOU対策として同じpreflightを
/// 再実行し(`state::ownership::apply_ownership_transfer`と同じパターン)、
/// 以降は全て再検証済みの新鮮な値のみを使う。
///
/// `causing_war_id`が`Some`の場合、そのWarはこの関数の戦争清掃ループから
/// 明示的に除外する(呼び出し元が自身の講和フローで既に/これから終結させる)。
/// 同様に`causing_crisis_id`が`Some`の場合、そのCrisisはCrisis清掃ループ
/// から除外する(`accept_demand`が受諾成功として自身で`ResolvedPeacefully`
/// へ遷移させるCrisis自身を、ここで`CancelledByCountryElimination`へ
/// 誤って上書きしないため)。
///
/// **原子性(§6)**: 州移転自体のTOCTOU再検証に加え、消滅国家が主要参加国である
/// 他Warの終結が全て成立可能であることを、一切の変更を加える前(ステップ0)で
/// 検証する。これは`execute_peace_settlement`が失敗し得る唯一の経路
/// (Crisis由来lineage不整合)を事前に潰すためで、これ以降のステップ(州移転・
/// 状態遷移・軍事/戦争/外交/Crisis/Claim/正当化清掃)はいずれも失敗し得ない
/// 純粋な代入・フィルタ操作のみで構成される。したがって、この関数が`Err`を
/// 返す場合、渡された全Registryは呼び出し前の状態から一切変更されていない。
#[allow(clippy::too_many_arguments)]
pub fn apply_country_elimination(
    plan: CountryEliminationPlan,
    causing_war_id: Option<WarId>,
    causing_crisis_id: Option<DiplomaticCrisisId>,
    current_date_str: &str,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
    military_registry: &mut MilitaryRegistry,
    army_registry: &mut ArmyRegistry,
    frontline_registry: &mut FrontlineRegistry,
    war_registry: &mut WarRegistry,
    battle_registry: &mut BattleRegistry,
    diplomacy_registry: &mut DiplomacyRegistry,
    crisis_registry: &mut CrisisRegistry,
    claim_registry: &mut ClaimRegistry,
    justification_registry: &mut WarJustificationRegistry,
    power_registry: &mut CountryPowerRegistry,
) -> Result<CountryEliminationOutcome, CountryEliminationError> {
    let fresh = preflight_country_elimination(
        plan.state_transfer.state_id(),
        plan.eliminated_country,
        plan.annexed_by,
        &plan.eliminated_date,
        state_registry,
        country_registry,
    )?;

    let eliminated_country = fresh.eliminated_country;
    let annexed_by = fresh.annexed_by;

    // 0. アトミック性事前検証(§6): 一切の変更を加える前に、消滅国家が
    // 主要参加国であるcausing war以外の全Active Warについて、それぞれの
    // Crisis由来lineageが健全である(=`execute_peace_settlement`が実際に
    // 呼ばれた際に失敗しない)ことを確認する。1件でも不整合があれば、
    // ここで即座に`Err`を返しどのRegistryも変更しない(全か無かの原子性)。
    let mut sorted_war_ids: Vec<WarId> = war_registry.wars.keys().copied().collect();
    sorted_war_ids.sort_by_key(|w| w.0);

    let mut other_primary_war_ids = Vec::new();
    let mut supporter_only_war_ids = Vec::new();
    for war_id in &sorted_war_ids {
        if Some(*war_id) == causing_war_id {
            continue;
        }
        let Some(war) = war_registry.wars.get(war_id) else {
            continue;
        };
        if war.status != WarStatus::Active || !war.is_participant(eliminated_country) {
            continue;
        }

        let is_primary = war.primary_attacker_id() == eliminated_country
            || war.primary_defender_id() == eliminated_country;

        if is_primary {
            let attacker = war.primary_attacker_id();
            let defender = war.primary_defender_id();
            crate::war::peace::resolve_crisis_war_lineage(
                war,
                attacker,
                defender,
                crisis_registry,
                claim_registry,
                state_registry,
            )
            .map_err(CountryEliminationError::OtherWarSettlementPreflight)?;
            other_primary_war_ids.push(*war_id);
        } else {
            supporter_only_war_ids.push(*war_id);
        }
    }

    // ここまでは一切のRegistryを変更していない。以降の全ステップは、
    // 事前検証済みの入力だけを使う失敗し得ない操作のみで構成される。

    // 1. 最後の所有陸州を移転する。
    let transfer_outcome = ownership::apply_final_state_transfer_for_elimination(
        fresh.state_transfer,
        state_registry,
        country_registry,
    )
    .map_err(CountryEliminationError::StateTransfer)?;

    // 2. 国家の活動状態を遷移させる(物理削除は絶対に行わない)。
    if let Some(country) = country_registry.get_mut(eliminated_country) {
        country.status = CountryStatus::Eliminated {
            eliminated_date: fresh.eliminated_date.clone(),
            annexed_by,
        };
    }

    // 2.5 PowerTierランキングから即時除外する(§3ユーザー指摘対応:
    // 月次再評価を待つeventual consistencyは不可。消滅トランザクション内で
    // 即座に`ordered_country_ids()`/`get()`の両方から消える)。
    power_registry.remove_country(eliminated_country);

    // 3. Claim清掃: 消滅国家自身が請求者のActive Claimを無効化する
    // (対象州が譲渡されて「達成された」Consumedとは異なり、請求者自身が
    // もう存在しないため根拠として使いようがない)。§9の戦争清掃より先に
    // 行う——`execute_peace_settlement`内部の無条件`mark_consumed`呼び出しが
    // (`ClaimRegistry::mark_consumed`が`Invalidated`を降格させない安全策と
    // 合わせ、二重の防御になる)このClaimを誤って`Consumed`へ書き戻さない
    // ようにするため。
    let mut claims_invalidated = Vec::new();
    for claim in claim_registry.claims.values_mut() {
        if claim.claimant_country == eliminated_country && claim.status == ClaimStatus::Active {
            claim.status = ClaimStatus::Invalidated;
            claims_invalidated.push(claim.id);
        }
    }
    claims_invalidated.sort_by_key(|id| id.0);

    // 4. 軍事清掃: 残存する全Divisionを解散する(§8: 自動転籍は行わない)。
    // 消滅国家の募兵キュー自体(`CountryData.recruitment_queue`)も、Division同様
    // 存在する意味を失うため空にする(残したままだと、Save検証が「消滅国家に
    // 募兵中の項目が残っている」ことを不整合として拒否する)。
    if let Some(country) = country_registry.get_mut(eliminated_country) {
        country.recruitment_queue.clear();
    }

    let mut division_ids_to_remove: Vec<_> = military_registry
        .divisions
        .values()
        .filter(|d| d.owner == eliminated_country)
        .map(|d| d.id)
        .collect();
    division_ids_to_remove.sort_by_key(|id| id.0);
    let divisions_disbanded = division_ids_to_remove.len();
    for division_id in division_ids_to_remove {
        military_registry.remove_division(division_id);
    }
    army_registry.sanitize_references(military_registry);

    // 4.5 進行中戦闘の整理: 消滅国家の師団を解散した結果、進行中(Ongoing)の
    // 戦闘が参照していたDivisionが消えることがある。次回の日次戦闘処理
    // (`military::update::process_daily_battles`)を待たず、この消滅
    // トランザクション内で直ちに整合させる——`save::validate::validate_battles`
    // はOngoing状態の戦闘が実在しないDivisionを参照していることを拒否する
    // ため、放置すると消滅直後にSaveした場合だけ検証エラーになる非決定的な
    // 挙動になってしまう。いずれかの陣営が空になった戦闘は中止する
    // (`execute_peace_settlement`の戦争終了時battle中止処理と同じ規則)。
    for battle in battle_registry.battles.values_mut() {
        if battle.status != crate::military::battle::BattleStatus::Ongoing {
            continue;
        }
        battle
            .attacker_division_ids
            .retain(|id| military_registry.divisions.contains_key(id));
        battle
            .defender_division_ids
            .retain(|id| military_registry.divisions.contains_key(id));
        if battle.attacker_division_ids.is_empty() || battle.defender_division_ids.is_empty() {
            battle.status = crate::military::battle::BattleStatus::Cancelled;
        }
    }

    // 4.6 占領支配の整理(§4/§5ユーザー指摘対応): 消滅国家が(支持国として
    // 参加していたWar経由も含め)占領支配者になっていた州があれば、法的
    // 所有国の支配へ戻す。causing war/他の主要参加War自身が終結する州は
    // 後続ステップ(`execute_peace_settlement`内の占領正常化)で改めて処理
    // されるため、ここでの対象は主に「Warは継続するが消滅国家が占領者
    // だった」支持国ケース(§4末尾要求)である——`execute_peace_settlement`
    // が触れない範囲を、ここで先に安全側へ倒しておく。
    for state in state_registry.states.iter_mut() {
        if state.controller_country == Some(eliminated_country) {
            state.controller_country = Some(state.owner_country_id);
            state.occupation_progress = 0.0;
            state.original_owner = None;
            state.war_id = None;
        }
    }

    // 5. 戦争清掃(§9): ステップ0で確定した2つのリストをそのまま使う
    // (この関数の実行中、他にRegistryを変更する主体は存在しないため、
    // ステップ0の判定はここでも有効)。
    let mut other_wars_ended = Vec::new();
    for war_id in other_primary_war_ids {
        end_other_primary_war_due_to_elimination(
            war_id,
            eliminated_country,
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
        other_wars_ended.push(war_id);
    }

    let mut wars_supporter_removed = Vec::new();
    for war_id in supporter_only_war_ids {
        if let Some(w) = war_registry.wars.get_mut(&war_id) {
            w.attackers.remove(&eliminated_country);
            w.defenders.remove(&eliminated_country);
        }
        wars_supporter_removed.push(war_id);
    }
    frontline_registry.sanitize_references(military_registry, war_registry);

    // 6. 外交清掃: 消滅国家が絡む二国間関係の条約・休戦・進行中活動を解除する
    // (`WarRegistry::are_countries_at_war`はWar自体から live 計算されるため、
    // ここで別途「敵対状態」を消す処理は不要――§9のWar清掃だけで十分)。
    for (key, relation) in diplomacy_registry.relations.iter_mut() {
        if key.0 == eliminated_country || key.1 == eliminated_country {
            relation.treaties.clear();
            relation.truce_until = None;
            relation.active_activity = None;
            relation.has_military_access = false;
        }
    }

    // 7. Crisis清掃: 消滅国家が当事国の未解決Crisisを強制終端する。
    let mut crises_cancelled = Vec::new();
    for crisis in crisis_registry.crises.values_mut() {
        if Some(crisis.id) != causing_crisis_id
            && (crisis.initiator == eliminated_country || crisis.target == eliminated_country)
            && is_unresolved_phase(crisis.current_phase)
        {
            crisis.current_phase = CrisisPhase::CancelledByCountryElimination;
            crises_cancelled.push(crisis.id);
        }
    }
    crises_cancelled.sort_by_key(|id| id.0);

    // 7.5. 支持表明清掃: 消滅国家が当事国(initiator/target)ではなく、単なる第三国支持国
    // としてのみ関与しているCrisisは(上のループでは)終端されない――そのCrisisは他の
    // 当事国にとって未解決のまま継続するのが正しい。しかし消滅国家自身の
    // `third_party_reactions`エントリは、もはや実在しない国家の支持表明として残り続けて
    // しまうため、当事国かどうかに関わらず全Crisisから明示的に除去する。
    for crisis in crisis_registry.crises.values_mut() {
        crisis.third_party_reactions.remove(&eliminated_country);
    }

    // 8. 正当化清掃: 消滅国家を要求国/対象国とする正当化を削除する。
    let mut justifications_removed: Vec<usize> = justification_registry
        .justifications
        .iter()
        .filter(|(_, j)| j.initiator == eliminated_country || j.target == eliminated_country)
        .map(|(id, _)| *id)
        .collect();
    justifications_removed.sort_unstable();
    for id in &justifications_removed {
        justification_registry.justifications.remove(id);
    }

    other_wars_ended.sort_by_key(|w| w.0);
    wars_supporter_removed.sort_by_key(|w| w.0);

    Ok(CountryEliminationOutcome {
        eliminated_country,
        annexed_by,
        final_state_id: transfer_outcome.state_id,
        eliminated_date: fresh.eliminated_date,
        divisions_disbanded,
        other_wars_ended,
        wars_supporter_removed,
        crises_cancelled,
        claims_invalidated,
        justifications_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::CountryData;
    use crate::diplomacy::claims::{ClaimSource, TerritorialClaim};
    use crate::diplomacy::crisis::{DiplomaticCrisis, ThirdCountryReaction, WarGoal, WarGoalType};
    use crate::diplomacy::data::{ActiveTreaty, DiplomaticPairKey, DiplomaticRelation, TreatyType};
    use crate::military::battle::{Battle, BattleStatus};
    use crate::military::data::{Division, DivisionSize, DivisionStatus, DivisionType};
    use crate::state::data::{StateData, StateRegistry};
    use crate::war::data::War;
    use crate::war::justification::WarJustification;
    use std::collections::HashSet;

    fn country(id: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            capital_state_id: StateId(999),
            ..CountryData::default()
        }
    }

    /// `id`国が`state_id`(唯一の所有陸州、首都)だけを所有する環境。
    /// `annexer_id`は陸州を1つ以上所有するActiveな別国。
    fn last_state_env(
        eliminated_id: usize,
        annexer_id: usize,
        state_id: usize,
    ) -> (StateRegistry, CountryRegistry) {
        let states = StateRegistry::build(vec![
            StateData {
                id: StateId(state_id),
                owner_country_id: CountryId(eliminated_id),
                is_sea: false,
                ..Default::default()
            },
            StateData {
                id: StateId(state_id + 100),
                owner_country_id: CountryId(annexer_id),
                is_sea: false,
                ..Default::default()
            },
        ]);
        let countries = CountryRegistry {
            countries: vec![
                CountryData {
                    id: CountryId(eliminated_id),
                    capital_state_id: StateId(state_id),
                    ..CountryData::default()
                },
                CountryData {
                    id: CountryId(annexer_id),
                    capital_state_id: StateId(state_id + 100),
                    ..CountryData::default()
                },
            ],
        };
        (states, countries)
    }

    fn division(id: usize, owner: usize, current_state: usize) -> Division {
        Division {
            id: crate::common::DivisionId(id),
            owner: CountryId(owner),
            division_type: DivisionType::Infantry,
            size: DivisionSize::Standard,
            current_state: StateId(current_state),
            destination: None,
            current_path: vec![],
            target_state: None,
            manpower: 10_000,
            max_manpower: 10_000,
            equipment: 100.0,
            max_equipment: 100.0,
            organization: 100.0,
            max_organization: 100.0,
            morale: 100.0,
            max_morale: 100.0,
            experience: 0.0,
            supply_ratio: 1.0,
            movement_progress: 0.0,
            status: DivisionStatus::Idle,
            def_id: crate::common::DivisionDefinitionId(0),
            attack_power: 10,
            defense_power: 10,
            combat_id: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn war(id: usize, attackers: &[usize], defenders: &[usize], target_state: usize) -> War {
        War {
            id: WarId(id),
            name: "Test War".to_string(),
            attackers: attackers.iter().map(|&c| CountryId(c)).collect(),
            defenders: defenders.iter().map(|&c| CountryId(c)).collect(),
            primary_attacker: Some(CountryId(attackers[0])),
            primary_defender: Some(CountryId(defenders[0])),
            source_crisis_id: None,
            war_goals: vec![WarGoal {
                attacker: CountryId(attackers[0]),
                defender: CountryId(defenders[0]),
                goal_type: WarGoalType::ConquerState,
                target_states: vec![StateId(target_state)],
                base_peace_cost: 20.0,
                international_concern: 10.0,
                completion: 0.0,
                is_primary: true,
            }],
            start_date: "1800/01/01".to_string(),
            end_date: None,
            duration_days: 0,
            war_score: 0.0,
            attacker_war_exhaustion: 0.0,
            defender_war_exhaustion: 0.0,
            occupied_states: HashSet::new(),
            status: crate::war::data::WarStatus::Active,
            winner: None,
            end_reason: None,
            applied_terms: Vec::new(),
            won_attacker_battles: 0,
            won_defender_battles: 0,
            processed_battle_ids: HashSet::new(),
        }
    }

    fn claim(
        id: usize,
        claimant: usize,
        target_state: usize,
        status: ClaimStatus,
    ) -> TerritorialClaim {
        TerritorialClaim {
            id: ClaimId(id),
            claimant_country: CountryId(claimant),
            target_state: StateId(target_state),
            strength: 50.0,
            created_date: "1800/01/01".to_string(),
            is_permanent: false,
            source: ClaimSource::Strategic,
            status,
        }
    }

    fn unresolved_crisis(id: usize, initiator: usize, target: usize) -> DiplomaticCrisis {
        DiplomaticCrisis {
            id: DiplomaticCrisisId(id),
            initiator: CountryId(initiator),
            target: CountryId(target),
            war_goals: vec![],
            start_date: "1800/01/01".to_string(),
            current_phase: CrisisPhase::DemandSent,
            escalation: 0.0,
            initiator_support: 0.0,
            target_resistance: 0.0,
            days_in_phase: 0,
            deadline_date: Some("1800/01/31".to_string()),
            international_concern: 0.0,
            third_party_reactions: Default::default(),
            related_claim_id: None,
            related_justification_id: None,
            related_war_id: None,
        }
    }

    fn justification(id: usize, initiator: usize, target: usize) -> WarJustification {
        WarJustification {
            id,
            initiator: CountryId(initiator),
            target: CountryId(target),
            target_state: StateId(0),
            start_date: "1800/01/01".to_string(),
            required_days: 30,
            days_passed: 30,
            is_ready: true,
            source_crisis_id: None,
            committed_attackers: vec![],
            committed_defenders: vec![],
        }
    }

    #[test]
    fn preflight_rejects_when_eliminated_country_does_not_exist() {
        let (states, countries) = last_state_env(1, 2, 1);
        let result = preflight_country_elimination(
            StateId(1),
            CountryId(99),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        );
        assert_eq!(
            result,
            Err(CountryEliminationError::StateTransfer(
                OwnershipTransferError::CountryNotFound
            ))
        );
    }

    #[test]
    fn preflight_rejects_when_already_eliminated() {
        let (states, mut countries) = last_state_env(1, 2, 1);
        countries.get_mut(CountryId(1)).unwrap().status = CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".to_string(),
            annexed_by: CountryId(2),
        };
        let result = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        );
        assert_eq!(result, Err(CountryEliminationError::AlreadyEliminated));
    }

    #[test]
    fn preflight_rejects_when_annexer_is_not_active() {
        let (states, mut countries) = last_state_env(1, 2, 1);
        countries.get_mut(CountryId(2)).unwrap().status = CountryStatus::Eliminated {
            eliminated_date: "1800/01/01".to_string(),
            annexed_by: CountryId(1),
        };
        let result = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        );
        assert_eq!(result, Err(CountryEliminationError::AnnexerNotActive));
    }

    #[test]
    fn preflight_and_apply_transfer_the_final_state_and_mark_the_country_eliminated() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let outcome = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.eliminated_country, CountryId(1));
        assert_eq!(outcome.annexed_by, CountryId(2));
        assert_eq!(outcome.final_state_id, StateId(1));
        assert_eq!(
            states.get(StateId(1)).unwrap().owner_country_id,
            CountryId(2)
        );

        let eliminated = countries.get(CountryId(1)).unwrap();
        assert!(eliminated.is_eliminated());
        assert_eq!(
            eliminated.status,
            CountryStatus::Eliminated {
                eliminated_date: "1800/06/01".to_string(),
                annexed_by: CountryId(2),
            }
        );
        // 物理削除しない: CountryRegistryには依然として2件存在する。
        assert_eq!(countries.countries.len(), 2);
    }

    #[test]
    fn apply_disbands_all_remaining_divisions_of_the_eliminated_country() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut military = MilitaryRegistry::default();
        military.add_division(division(0, 1, 1));
        military.add_division(division(1, 1, 1));
        military.add_division(division(2, 2, 101)); // annexer's own division, must survive
        let mut army = ArmyRegistry::default();
        army.create_army(
            CountryId(1),
            &[crate::common::DivisionId(0), crate::common::DivisionId(1)],
            &military,
        );

        let outcome = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut military,
            &mut army,
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.divisions_disbanded, 2);
        assert!(!military.divisions.is_empty());
        assert!(
            !military.divisions.values().any(|d| d.owner == CountryId(1)),
            "no division owned by the eliminated country may remain"
        );
        assert!(
            military
                .divisions
                .contains_key(&crate::common::DivisionId(2)),
            "the annexer's own division must be untouched"
        );
        assert!(
            army.armies.is_empty(),
            "the eliminated country's army must be auto-disbanded once empty"
        );
    }

    #[test]
    fn apply_ends_other_active_wars_where_eliminated_country_is_a_primary_participant() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut war_registry = WarRegistry::default();
        // 消滅原因となったWar(causing war)。清掃ループから除外される。
        war_registry.wars.insert(WarId(1), war(1, &[2], &[1], 1));
        // 消滅国家が主要参加国(defender)である、無関係な別のWar。
        war_registry.wars.insert(WarId(2), war(2, &[9], &[1], 101));

        let outcome = apply_country_elimination(
            plan,
            Some(WarId(1)),
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut war_registry,
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.other_wars_ended, vec![WarId(2)]);
        assert_eq!(
            war_registry.wars.get(&WarId(1)).unwrap().status,
            crate::war::data::WarStatus::Active,
            "the causing war must be left untouched for the caller to finish settling"
        );
        let war2 = war_registry.wars.get(&WarId(2)).unwrap();
        assert_eq!(
            war2.status,
            crate::war::data::WarStatus::AttackerVictory,
            "the eliminated country was the defender, so the attacker must be recorded as the \
             winner (not a white-peace draw)"
        );
        assert_eq!(war2.winner, Some(CountryId(9)));
        assert_eq!(
            war2.applied_terms.last().map(String::as_str),
            Some("peace_term.defender_eliminated")
        );
    }

    #[test]
    fn apply_only_removes_the_eliminated_country_from_a_war_it_only_supported() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut war_registry = WarRegistry::default();
        war_registry.wars.insert(WarId(1), war(1, &[2], &[1], 1));
        // 消滅国家(1)が支持国(非主要参加国)としてのみ参加する、無関係な別のWar。
        let mut supported_war = war(2, &[9], &[8], 101);
        supported_war.attackers.insert(CountryId(1));
        war_registry.wars.insert(WarId(2), supported_war);

        // P21-021再監査対応(§3/§5): 支持国のみとして参加するWar(2)であっても、
        // 消滅国家の師団・戦闘・前線・占領への参照は完全に清掃されることを検証する。
        let mut military_registry = MilitaryRegistry::default();
        // `add_division`は渡した`Division.id`を無視し、自身のnext_idカウンタで
        // 正規のIdを発行する(feedback_test_registry_id_assignment参照)ため、
        // 戻り値を必ず使う——ここでリテラルDivisionIdを決め打ちすると、以降の
        // Battle/FrontlineへのId埋め込みが実在しないIdを参照してしまう。
        let eliminated_division_id = military_registry.add_division(division(3, 1, 101));
        // 敵側(8)の師団は生存させ、「攻撃側だけが消滅国家の解散で空になる」ことを
        // 実際に証明する(両陣営とも最初から空だと、原因を証明したことにならない)。
        let surviving_division_id = military_registry.add_division(division(4, 8, 101));

        let mut battle_registry = BattleRegistry::default();
        battle_registry.battles.insert(
            crate::common::BattleId(0),
            Battle {
                id: crate::common::BattleId(0),
                war_id: WarId(2),
                state_id: StateId(101),
                attacker_country: CountryId(1),
                defender_country: CountryId(8),
                attacker_division_ids: vec![eliminated_division_id],
                defender_division_ids: vec![surviving_division_id],
                attacker_origins: std::collections::HashMap::new(),
                start_date: "1800/05/01".to_string(),
                elapsed_days: 1,
                status: BattleStatus::Ongoing,
            },
        );

        let mut frontline_registry = FrontlineRegistry::default();
        frontline_registry.frontlines.insert(
            crate::common::FrontlineId(0),
            crate::war::frontline::Frontline {
                frontline_id: crate::common::FrontlineId(0),
                war_id: WarId(2),
                attacker_country_id: CountryId(9),
                defender_country_id: CountryId(8),
                attacker_front_regions: Vec::new(),
                defender_front_regions: Vec::new(),
                border_region_pairs: Vec::new(),
            },
        );
        let mut fl_plan =
            crate::war::frontline::FrontlinePlan::new(crate::common::FrontlineId(0), CountryId(9));
        fl_plan.assigned_division_ids = vec![eliminated_division_id];
        frontline_registry
            .plans
            .insert((crate::common::FrontlineId(0), CountryId(9)), fl_plan);
        frontline_registry
            .division_frontline_map
            .insert(eliminated_division_id, crate::common::FrontlineId(0));

        // state101(annexer 2の所有)を消滅国家(1)が占領支配しているものとする。
        states.get_mut(StateId(101)).unwrap().controller_country = Some(CountryId(1));

        let outcome = apply_country_elimination(
            plan,
            Some(WarId(1)),
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut military_registry,
            &mut ArmyRegistry::default(),
            &mut frontline_registry,
            &mut war_registry,
            &mut battle_registry,
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.wars_supporter_removed, vec![WarId(2)]);
        let war2 = war_registry.wars.get(&WarId(2)).unwrap();
        assert_eq!(
            war2.status,
            crate::war::data::WarStatus::Active,
            "a war the eliminated country only supported must remain active for its real participants"
        );
        assert!(
            !war2.attackers.contains(&CountryId(1)),
            "the eliminated country must be removed from the participant set"
        );
        assert!(war2.attackers.contains(&CountryId(9)));

        assert!(
            !military_registry
                .divisions
                .values()
                .any(|d| d.owner == CountryId(1)),
            "the eliminated country's division must be disbanded even though its war continues"
        );
        let battle = battle_registry
            .battles
            .get(&crate::common::BattleId(0))
            .unwrap();
        assert!(
            !battle
                .attacker_division_ids
                .contains(&eliminated_division_id),
            "the disbanded division's id must be removed from the ongoing battle"
        );
        assert!(
            battle.attacker_division_ids.is_empty(),
            "the attacker side must be left with no participants"
        );
        assert_eq!(
            battle.defender_division_ids,
            vec![surviving_division_id],
            "the surviving defender division must be untouched"
        );
        assert_eq!(
            battle.status,
            BattleStatus::Cancelled,
            "the battle must be cancelled once the eliminated country's side has no participants left"
        );
        assert!(
            !frontline_registry
                .division_frontline_map
                .contains_key(&eliminated_division_id),
            "the disbanded division must be removed from the frontline division map"
        );
        assert!(
            !frontline_registry
                .plans
                .get(&(crate::common::FrontlineId(0), CountryId(9)))
                .unwrap()
                .assigned_division_ids
                .contains(&eliminated_division_id),
            "the disbanded division must be removed from the frontline plan's assignment"
        );
        assert_eq!(
            states.get(StateId(101)).unwrap().controller_country,
            Some(CountryId(2)),
            "occupation by the eliminated country must be released back to the legal owner \
             even though the underlying war it supported continues"
        );
    }

    #[test]
    fn apply_invalidates_the_eliminated_countrys_own_active_claims_but_not_others() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut claim_registry = ClaimRegistry::default();
        claim_registry
            .claims
            .insert(ClaimId(0), claim(0, 1, 101, ClaimStatus::Active));
        claim_registry
            .claims
            .insert(ClaimId(1), claim(1, 2, 1, ClaimStatus::Active));

        let outcome = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut claim_registry,
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.claims_invalidated, vec![ClaimId(0)]);
        assert_eq!(
            claim_registry.claims.get(&ClaimId(0)).unwrap().status,
            ClaimStatus::Invalidated
        );
        assert_eq!(
            claim_registry.claims.get(&ClaimId(1)).unwrap().status,
            ClaimStatus::Active,
            "a claim not made by the eliminated country must not be touched"
        );
    }

    #[test]
    fn apply_cancels_unresolved_crises_involving_the_eliminated_country_except_the_causing_one() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut crisis_registry = CrisisRegistry::default();
        crisis_registry
            .crises
            .insert(DiplomaticCrisisId(0), unresolved_crisis(0, 2, 1));
        crisis_registry
            .crises
            .insert(DiplomaticCrisisId(1), unresolved_crisis(1, 9, 1));

        let outcome = apply_country_elimination(
            plan,
            None,
            Some(DiplomaticCrisisId(0)),
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut crisis_registry,
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.crises_cancelled, vec![DiplomaticCrisisId(1)]);
        assert_eq!(
            crisis_registry
                .crises
                .get(&DiplomaticCrisisId(0))
                .unwrap()
                .current_phase,
            CrisisPhase::DemandSent,
            "the causing crisis must be left for the caller (accept_demand) to resolve"
        );
        assert_eq!(
            crisis_registry
                .crises
                .get(&DiplomaticCrisisId(1))
                .unwrap()
                .current_phase,
            CrisisPhase::CancelledByCountryElimination
        );
    }

    /// P21-021再監査対応: 消滅国家がinitiator/targetではなく単なる第三国支持国として
    /// のみ関与しているCrisisは終端されない(他の当事国にとって未解決のまま継続する)が、
    /// 消滅国家自身の`third_party_reactions`エントリだけは全Crisisから明示的に除去される。
    #[test]
    fn apply_removes_the_eliminated_countrys_third_party_support_but_leaves_the_crisis_unresolved()
    {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut crisis_registry = CrisisRegistry::default();
        let mut third_party_crisis = unresolved_crisis(0, 9, 10);
        third_party_crisis
            .third_party_reactions
            .insert(CountryId(1), ThirdCountryReaction::SupportsInitiator);
        third_party_crisis
            .third_party_reactions
            .insert(CountryId(2), ThirdCountryReaction::SupportsTarget);
        crisis_registry
            .crises
            .insert(DiplomaticCrisisId(0), third_party_crisis);

        let outcome = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut crisis_registry,
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert!(
            outcome.crises_cancelled.is_empty(),
            "a crisis the eliminated country only supports (not initiator/target) must not be cancelled"
        );
        let crisis = crisis_registry.crises.get(&DiplomaticCrisisId(0)).unwrap();
        assert_eq!(
            crisis.current_phase,
            CrisisPhase::DemandSent,
            "the crisis must remain unresolved for its real initiator/target"
        );
        assert!(
            !crisis.third_party_reactions.contains_key(&CountryId(1)),
            "the eliminated country's own support entry must be removed"
        );
        assert_eq!(
            crisis.third_party_reactions.get(&CountryId(2)),
            Some(&ThirdCountryReaction::SupportsTarget),
            "an unrelated supporter's entry must survive"
        );
    }

    #[test]
    fn apply_removes_justifications_naming_the_eliminated_country() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut justification_registry = WarJustificationRegistry::default();
        justification_registry
            .justifications
            .insert(0, justification(0, 1, 9));
        justification_registry
            .justifications
            .insert(1, justification(1, 9, 2));

        let outcome = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut justification_registry,
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert_eq!(outcome.justifications_removed, vec![0]);
        assert!(justification_registry.justifications.contains_key(&1));
    }

    #[test]
    fn apply_clears_the_eliminated_countrys_recruitment_queue() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        countries
            .get_mut(CountryId(1))
            .unwrap()
            .recruitment_queue
            .push(crate::military::recruitment::RecruitmentQueueItem {
                division_id: crate::common::DivisionDefinitionId(0),
                target_state: StateId(1),
                days_remaining: 5,
                total_days: 10,
            });
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert!(
            countries
                .get(CountryId(1))
                .unwrap()
                .recruitment_queue
                .is_empty(),
            "the eliminated country's recruitment queue must be cleared"
        );
    }

    /// 要求(P21-021 §5): 消滅国家の師団を含む進行中戦闘は、次回の日次戦闘処理を
    /// 待たず消滅トランザクション内で直ちに整理される——参照が消えたDivisionは
    /// 除去し、いずれかの陣営が空になれば戦闘自体を中止する。
    #[test]
    fn apply_immediately_sanitizes_ongoing_battles_referencing_disbanded_divisions() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut military_registry = MilitaryRegistry::default();
        military_registry.add_division(division(0, 1, 1)); // 消滅国家(1)の師団
        military_registry.add_division(division(1, 2, 101)); // 併合先(2)の師団、生存

        let mut battle_registry = BattleRegistry::default();
        battle_registry.battles.insert(
            crate::common::BattleId(0),
            Battle {
                id: crate::common::BattleId(0),
                war_id: WarId(1),
                state_id: StateId(1),
                attacker_country: CountryId(2),
                defender_country: CountryId(1),
                attacker_division_ids: vec![crate::common::DivisionId(1)],
                defender_division_ids: vec![crate::common::DivisionId(0)],
                attacker_origins: std::collections::HashMap::new(),
                start_date: "1800/05/01".to_string(),
                elapsed_days: 1,
                status: BattleStatus::Ongoing,
            },
        );

        apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut military_registry,
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut battle_registry,
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        let battle = battle_registry
            .battles
            .get(&crate::common::BattleId(0))
            .unwrap();
        assert!(
            !battle
                .defender_division_ids
                .contains(&crate::common::DivisionId(0)),
            "the disbanded division's id must be removed from the battle's participant list"
        );
        assert_eq!(
            battle.status,
            BattleStatus::Cancelled,
            "the battle must be cancelled once the defending side has no participants left"
        );
    }

    #[test]
    fn apply_clears_diplomatic_treaties_and_truces_involving_the_eliminated_country() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut diplomacy_registry = DiplomacyRegistry::default();
        let key = DiplomaticPairKey::new(CountryId(1), CountryId(9)).unwrap();
        diplomacy_registry.relations.insert(
            key,
            DiplomaticRelation {
                treaties: vec![ActiveTreaty {
                    treaty_type: TreatyType::Alliance,
                    countries: (CountryId(1), CountryId(9)),
                    signed_date: "1800/01/01".to_string(),
                    is_active: true,
                }],
                truce_until: Some("1900/01/01".to_string()),
                has_military_access: true,
                active_activity: Some(crate::diplomacy::data::ActiveDiplomaticActivity {
                    activity_type: crate::diplomacy::data::DiplomaticActivityType::ImproveRelations,
                    initiator: CountryId(1),
                    target: CountryId(9),
                    days_remaining: 10,
                    daily_opinion_change: 1.0,
                }),
                ..Default::default()
            },
        );
        let unrelated_key = DiplomaticPairKey::new(CountryId(9), CountryId(2)).unwrap();
        diplomacy_registry.relations.insert(
            unrelated_key,
            DiplomaticRelation {
                truce_until: Some("1900/01/01".to_string()),
                ..Default::default()
            },
        );

        apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut diplomacy_registry,
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        )
        .unwrap();

        assert!(
            diplomacy_registry
                .relations
                .get(&key)
                .unwrap()
                .treaties
                .is_empty()
        );
        assert!(
            diplomacy_registry
                .relations
                .get(&key)
                .unwrap()
                .truce_until
                .is_none()
        );
        assert!(
            !diplomacy_registry
                .relations
                .get(&key)
                .unwrap()
                .has_military_access,
            "has_military_access must be cleared for a relation involving the eliminated country"
        );
        assert!(
            diplomacy_registry
                .relations
                .get(&key)
                .unwrap()
                .active_activity
                .is_none(),
            "an in-progress diplomatic activity must be cancelled"
        );
        assert!(
            diplomacy_registry
                .relations
                .get(&unrelated_key)
                .unwrap()
                .truce_until
                .is_some(),
            "a relation not involving the eliminated country must not be touched"
        );
    }

    #[test]
    fn apply_rejects_a_stale_plan_when_the_state_owner_changed_after_preflight() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        // preflightとapplyの間に、対象州の所有者が第三国へ変わった(TOCTOU)。
        countries.countries.push(country(9));
        states.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(9);

        let result = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut CountryPowerRegistry::default(),
        );

        assert_eq!(
            result,
            Err(CountryEliminationError::StateTransfer(
                OwnershipTransferError::CurrentOwnerMismatch
            ))
        );
        assert_eq!(
            states.get(StateId(1)).unwrap().owner_country_id,
            CountryId(9),
            "a rejected apply must not mutate state ownership"
        );
    }

    /// 要求(P21-021 §3): 消滅トランザクション自体がPowerTierランキングから
    /// 即時除外する(月次再評価を待たない)。
    #[test]
    fn apply_immediately_excludes_the_eliminated_country_from_the_power_registry() {
        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        // 消滅前に実際に評価済みで、CountryId(1)がランキングに存在する
        // 状態を`evaluate_country_power`(実際の月次評価と同じ関数)で再現する。
        let mut power_registry = crate::country::power::evaluate_country_power(
            &countries,
            &states,
            &MilitaryRegistry::default(),
            &crate::building::data::BuildingRegistry::default(),
            "1800/01/01".to_string(),
        );
        assert!(
            power_registry.get(CountryId(1)).is_some(),
            "test setup must actually have CountryId(1) in the pre-elimination ranking"
        );

        apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut MilitaryRegistry::default(),
            &mut ArmyRegistry::default(),
            &mut FrontlineRegistry::default(),
            &mut WarRegistry::default(),
            &mut BattleRegistry::default(),
            &mut DiplomacyRegistry::default(),
            &mut CrisisRegistry::default(),
            &mut ClaimRegistry::default(),
            &mut WarJustificationRegistry::default(),
            &mut power_registry,
        )
        .unwrap();

        assert!(
            power_registry.get(CountryId(1)).is_none(),
            "the eliminated country must not have a power assessment immediately after elimination"
        );
        assert!(
            !power_registry.ordered_country_ids().contains(&CountryId(1)),
            "the eliminated country must not appear in the ranking list immediately"
        );
    }

    /// 要求(P21-021 §6アトミック性): 消滅国家が主要参加国である「他War」の
    /// うち1件が壊れたCrisis由来lineage(存在しないCrisisを参照する
    /// `source_crisis_id`)を持つ場合、`apply_country_elimination`全体が
    /// エラーを返し、渡した全Registryが呼び出し前と完全に一致したまま
    /// 変更されない(部分適用を残さない)ことを、主要フィールドの
    /// スナップショット比較で証明する。
    #[test]
    fn apply_rejects_atomically_when_another_primary_war_has_a_broken_crisis_lineage() {
        #[derive(Debug, PartialEq)]
        struct Snapshot {
            state1_owner: CountryId,
            state101_controller: Option<CountryId>,
            country1_status: CountryStatus,
            country1_recruitment_queue_len: usize,
            war2_status: crate::war::data::WarStatus,
            war2_attackers: std::collections::HashSet<CountryId>,
            division_ids: Vec<crate::common::DivisionId>,
            claim1_status: ClaimStatus,
            crisis1_phase: CrisisPhase,
            justification_ids: Vec<usize>,
            army_for_division0: Option<crate::common::ArmyId>,
            army_member_ids: Vec<crate::common::DivisionId>,
            frontline_division_map: Option<crate::common::FrontlineId>,
            frontline_plan_assigned: Vec<crate::common::DivisionId>,
            battle_status: BattleStatus,
            battle_defender_division_ids: Vec<crate::common::DivisionId>,
            treaty_count: usize,
            power_assessment_present: bool,
        }

        #[allow(clippy::too_many_arguments)]
        fn snapshot(
            states: &StateRegistry,
            countries: &CountryRegistry,
            war_registry: &WarRegistry,
            military_registry: &MilitaryRegistry,
            claim_registry: &ClaimRegistry,
            crisis_registry: &CrisisRegistry,
            justification_registry: &WarJustificationRegistry,
            army_registry: &ArmyRegistry,
            frontline_registry: &FrontlineRegistry,
            battle_registry: &BattleRegistry,
            diplomacy_registry: &DiplomacyRegistry,
            power_registry: &CountryPowerRegistry,
        ) -> Snapshot {
            let mut division_ids: Vec<_> = military_registry.divisions.keys().copied().collect();
            division_ids.sort_by_key(|id| id.0);
            let mut justification_ids: Vec<_> = justification_registry
                .justifications
                .keys()
                .copied()
                .collect();
            justification_ids.sort_unstable();
            let army_for_division0 = army_registry.army_for_division(crate::common::DivisionId(0));
            let mut army_member_ids = army_for_division0
                .and_then(|id| army_registry.armies.get(&id))
                .map(|g| g.member_division_ids.clone())
                .unwrap_or_default();
            army_member_ids.sort_by_key(|id| id.0);
            let battle = battle_registry
                .battles
                .get(&crate::common::BattleId(0))
                .unwrap();
            Snapshot {
                state1_owner: states.get(StateId(1)).unwrap().owner_country_id,
                state101_controller: states.get(StateId(101)).unwrap().controller_country,
                country1_status: countries.get(CountryId(1)).unwrap().status.clone(),
                country1_recruitment_queue_len: countries
                    .get(CountryId(1))
                    .unwrap()
                    .recruitment_queue
                    .len(),
                war2_status: war_registry.wars.get(&WarId(2)).unwrap().status,
                war2_attackers: war_registry.wars.get(&WarId(2)).unwrap().attackers.clone(),
                division_ids,
                claim1_status: claim_registry.claims.get(&ClaimId(0)).unwrap().status,
                crisis1_phase: crisis_registry
                    .crises
                    .get(&DiplomaticCrisisId(0))
                    .unwrap()
                    .current_phase,
                justification_ids,
                army_for_division0,
                army_member_ids,
                frontline_division_map: frontline_registry
                    .division_frontline_map
                    .get(&crate::common::DivisionId(0))
                    .copied(),
                frontline_plan_assigned: frontline_registry
                    .plans
                    .get(&(crate::common::FrontlineId(0), CountryId(1)))
                    .map(|p| p.assigned_division_ids.clone())
                    .unwrap_or_default(),
                battle_status: battle.status,
                battle_defender_division_ids: battle.defender_division_ids.clone(),
                treaty_count: diplomacy_registry
                    .get(CountryId(1), CountryId(3))
                    .map(|r| r.treaties.len())
                    .unwrap_or(0),
                power_assessment_present: power_registry.get(CountryId(1)).is_some(),
            }
        }

        let (mut states, mut countries) = last_state_env(1, 2, 1);
        let plan = preflight_country_elimination(
            StateId(1),
            CountryId(1),
            CountryId(2),
            "1800/06/01",
            &states,
            &countries,
        )
        .unwrap();

        let mut war_registry = WarRegistry::default();
        // war_a: 消滅国家(1)が主要参加国(defender)で、存在しないCrisisId(99)を
        // 参照する壊れたCrisis由来lineageを持つ「他War」。
        let mut broken_war = war(2, &[9], &[1], 101);
        broken_war.source_crisis_id = Some(DiplomaticCrisisId(99));
        war_registry.wars.insert(WarId(2), broken_war);

        let mut military_registry = MilitaryRegistry::default();
        military_registry.add_division(division(0, 1, 1));

        let mut claim_registry = ClaimRegistry::default();
        claim_registry
            .claims
            .insert(ClaimId(0), claim(0, 1, 101, ClaimStatus::Active));

        let mut crisis_registry = CrisisRegistry::default();
        crisis_registry
            .crises
            .insert(DiplomaticCrisisId(0), unresolved_crisis(0, 9, 1));

        let mut justification_registry = WarJustificationRegistry::default();
        justification_registry
            .justifications
            .insert(0, justification(0, 1, 9));

        // 占領: state101(併合先2の所有)を消滅国家(1)が支配しているものとする。
        states.get_mut(StateId(101)).unwrap().controller_country = Some(CountryId(1));

        // 募兵キュー: 消滅国家(1)自身の募兵キューに1件残す。
        countries
            .get_mut(CountryId(1))
            .unwrap()
            .recruitment_queue
            .push(crate::military::recruitment::RecruitmentQueueItem {
                division_id: crate::common::DivisionDefinitionId(0),
                target_state: StateId(1),
                days_remaining: 10,
                total_days: 30,
            });

        let mut army_registry = ArmyRegistry::default();
        army_registry
            .create_army(
                CountryId(1),
                &[crate::common::DivisionId(0)],
                &military_registry,
            )
            .expect("army creation must succeed for a real division");

        let mut frontline_registry = FrontlineRegistry::default();
        frontline_registry.frontlines.insert(
            crate::common::FrontlineId(0),
            crate::war::frontline::Frontline {
                frontline_id: crate::common::FrontlineId(0),
                war_id: WarId(2),
                attacker_country_id: CountryId(9),
                defender_country_id: CountryId(1),
                attacker_front_regions: Vec::new(),
                defender_front_regions: Vec::new(),
                border_region_pairs: Vec::new(),
            },
        );
        let mut fl_plan =
            crate::war::frontline::FrontlinePlan::new(crate::common::FrontlineId(0), CountryId(1));
        fl_plan.assigned_division_ids = vec![crate::common::DivisionId(0)];
        frontline_registry
            .plans
            .insert((crate::common::FrontlineId(0), CountryId(1)), fl_plan);
        frontline_registry
            .division_frontline_map
            .insert(crate::common::DivisionId(0), crate::common::FrontlineId(0));

        let mut battle_registry = BattleRegistry::default();
        battle_registry.battles.insert(
            crate::common::BattleId(0),
            Battle {
                id: crate::common::BattleId(0),
                war_id: WarId(2),
                state_id: StateId(101),
                attacker_country: CountryId(9),
                defender_country: CountryId(1),
                attacker_division_ids: Vec::new(),
                defender_division_ids: vec![crate::common::DivisionId(0)],
                attacker_origins: std::collections::HashMap::new(),
                start_date: "1800/05/01".to_string(),
                elapsed_days: 1,
                status: BattleStatus::Ongoing,
            },
        );

        let mut diplomacy_registry = DiplomacyRegistry::default();
        diplomacy_registry
            .get_or_create_mut(CountryId(1), CountryId(3))
            .unwrap()
            .treaties
            .push(ActiveTreaty {
                treaty_type: TreatyType::NonAggressionPact,
                countries: (CountryId(1), CountryId(3)),
                signed_date: "1800/01/01".to_string(),
                is_active: true,
            });

        // PowerRegistry: 実際の評価関数で消滅国家(1)の実データを構築する
        // (§3のテストと同じ手法。空のDefaultでは「何も変わらない」ことの証明にならない)。
        let mut power_registry = crate::country::power::evaluate_country_power(
            &countries,
            &states,
            &military_registry,
            &crate::building::data::BuildingRegistry::default(),
            "1800/01/01".to_string(),
        );
        assert!(
            power_registry.get(CountryId(1)).is_some(),
            "test setup must actually have CountryId(1) in the pre-elimination ranking"
        );

        let before = snapshot(
            &states,
            &countries,
            &war_registry,
            &military_registry,
            &claim_registry,
            &crisis_registry,
            &justification_registry,
            &army_registry,
            &frontline_registry,
            &battle_registry,
            &diplomacy_registry,
            &power_registry,
        );

        let result = apply_country_elimination(
            plan,
            None,
            None,
            "1800/06/01",
            &mut states,
            &mut countries,
            &mut military_registry,
            &mut army_registry,
            &mut frontline_registry,
            &mut war_registry,
            &mut battle_registry,
            &mut diplomacy_registry,
            &mut crisis_registry,
            &mut claim_registry,
            &mut justification_registry,
            &mut power_registry,
        );

        assert_eq!(
            result,
            Err(CountryEliminationError::OtherWarSettlementPreflight(
                "war_error.peace.crisis_lineage_missing_crisis"
            ))
        );

        let after = snapshot(
            &states,
            &countries,
            &war_registry,
            &military_registry,
            &claim_registry,
            &crisis_registry,
            &justification_registry,
            &army_registry,
            &frontline_registry,
            &battle_registry,
            &diplomacy_registry,
            &power_registry,
        );
        assert_eq!(
            before, after,
            "a rejected apply_country_elimination must leave every Registry completely \
             unchanged: state ownership/occupation, country status/recruitment queue, the \
             other war, divisions, the eliminated country's own claim, an unrelated crisis, \
             justifications, ArmyRegistry membership, FrontlineRegistry assignment, \
             BattleRegistry status/participants, DiplomacyRegistry treaties, and \
             CountryPowerRegistry"
        );
    }
}
