//! P21-020: 州の法的所有権(`StateData.owner_country_id`)を変更する、通常ゲームプレイ中の
//! 唯一の共通Domain API。
//!
//! 既存の`StateRegistry::transfer_region_ownership`は現在所有国を検証せず、
//! `CountryData.capital_state_id`も追従しない無条件API だった。P21-011の要求受諾・
//! P21-019のCrisis由来War講和・通常Warの講和がそれぞれ独自に(または全く)検証を
//! 行っており、通常Warの`CedeWarGoalRegion`には所有者不一致の保護が無い非対称が
//! あった。この API へ全経路を集約し、同じvalidation・同じ首都再配置規則・同じ
//! 原子性を適用する。
//!
//! occupationによる`controller_country`の変更(法的所有権は変わらない)はこの API の
//! 対象外――`StateData::controller()`が既存どおり`controller_country.unwrap_or(
//! owner_country_id)`で占領と所有を区別する。

use crate::common::{CountryId, StateId};
use crate::country::CountryRegistry;
use crate::state::data::StateRegistry;

/// 州所有権移転の失敗理由。
///
/// `CountryNotFound`は`expected_current_owner`・`new_owner`のどちらが存在しないかを
/// 区別しない――呼び出し元はいずれも「Crisis/War/Claimの参加国として既に検証済みの
/// 国家」を渡すため、実際にこのバリアントへ到達するのは異常データのみであり、UI表示上
/// どちらが欠けているかを分けて案内する実用上の必要が無い(呼び出し元の
/// `CountryNotFound`テストはexpected/new双方を個別に検証する)。
///
/// `LastOwnedLandState`は「首都を割譲するが残存陸州が無い」場合と「(首都かどうかに
/// 関わらず)割譲対象が最後の所有陸州そのものである」場合の両方を1つのバリアントで表す。
/// `CountryData.capital_state_id`は常に現所有陸州を指す不変条件(`app/loader.rs`が
/// 初期データで、この API 自身が実行時に維持する)があるため、「最後の所有陸州を失う」
/// ケースは構造的に必ず「首都を失うが再配置先が無い」ケースと一致する――
/// `CapitalReplacementUnavailable`のような別名を追加すると同じ実行時条件に2つの
/// 名前を与えることになり、かえって紛らわしいため統合した。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipTransferError {
    StateNotFound,
    SeaState,
    CountryNotFound,
    CurrentOwnerMismatch,
    SameOwner,
    LastOwnedLandState,
}

impl OwnershipTransferError {
    /// 表示用の翻訳キー(P20-009の規約)。UI側で`localization::t()`により言語ごとの
    /// 表示名へ解決する。
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::StateNotFound => "state_error.ownership.state_not_found",
            Self::SeaState => "state_error.ownership.sea_state",
            Self::CountryNotFound => "state_error.ownership.country_not_found",
            Self::CurrentOwnerMismatch => "state_error.ownership.current_owner_mismatch",
            Self::SameOwner => "state_error.ownership.same_owner",
            Self::LastOwnedLandState => "state_error.ownership.last_owned_land_state",
        }
    }
}

/// 州所有権移転が成功した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnershipTransferOutcome {
    pub state_id: StateId,
    pub previous_owner: CountryId,
    pub new_owner: CountryId,
    /// 首都再配置が発生した場合、`(旧首都国, 新しい capital_state_id)`。
    /// 対象州が旧所有国の首都でなければ`None`。
    pub capital_reassignment: Option<(CountryId, StateId)>,
}

/// 検証済みの州所有権移転計画(Registryを一切変更せずに構築される)。
///
/// `preflight_transfer_state_ownership`でのみ構築でき、`apply_ownership_transfer`
/// でのみ消費できる――検証と適用の間で同じ条件を再計算して食い違わせないための
/// 型(§7の`PeaceSettlementPlan`と同じ設計意図)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatedOwnershipTransfer {
    state_id: StateId,
    previous_owner: CountryId,
    new_owner: CountryId,
    capital_reassignment: Option<(CountryId, StateId)>,
}

impl ValidatedOwnershipTransfer {
    pub fn state_id(&self) -> StateId {
        self.state_id
    }

    pub fn previous_owner(&self) -> CountryId {
        self.previous_owner
    }

    pub fn new_owner(&self) -> CountryId {
        self.new_owner
    }

    pub fn capital_reassignment(&self) -> Option<(CountryId, StateId)> {
        self.capital_reassignment
    }
}

/// 対象州が現所有国の首都であれば、割譲後にその国が残す陸州のうち`StateId`昇順で
/// 最小の州を新首都候補として返す(HashMap挿入順に依存しない決定論)。首都でなければ
/// `Ok(None)`。有効な残存陸州が一つも無ければ`Err(LastOwnedLandState)`。
///
/// controllerではなくowner(法的所有)だけを候補選定に使う――第三国が占領中でも
/// 法的に所有していれば有効な首都候補になり得る、逆に自国が占領中でも法的に
/// 所有していなければ候補にしない。
fn plan_capital_reassignment(
    state_id: StateId,
    current_owner: CountryId,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
) -> Result<Option<(CountryId, StateId)>, OwnershipTransferError> {
    let Some(owner_country) = country_registry.get(current_owner) else {
        return Ok(None);
    };
    if owner_country.capital_state_id != state_id {
        return Ok(None);
    }

    let mut remaining: Vec<StateId> = state_registry
        .states
        .iter()
        .filter(|s| !s.is_sea && s.owner_country_id == current_owner && s.id != state_id)
        .map(|s| s.id)
        .collect();
    remaining.sort_by_key(|s| s.0);

    match remaining.first() {
        Some(&new_capital) => Ok(Some((current_owner, new_capital))),
        None => Err(OwnershipTransferError::LastOwnedLandState),
    }
}

/// 州所有権移転の事前検証(Registryを一切変更しない純粋関数)。§3の必須validationを
/// 全て順に確認し、成功すれば以降`apply_ownership_transfer`で確実に適用できる
/// `ValidatedOwnershipTransfer`を返す。
pub fn preflight_transfer_state_ownership(
    state_id: StateId,
    expected_current_owner: CountryId,
    new_owner: CountryId,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
) -> Result<ValidatedOwnershipTransfer, OwnershipTransferError> {
    let state = state_registry
        .get(state_id)
        .ok_or(OwnershipTransferError::StateNotFound)?;

    if state.is_sea {
        return Err(OwnershipTransferError::SeaState);
    }

    if country_registry.get(expected_current_owner).is_none() {
        return Err(OwnershipTransferError::CountryNotFound);
    }
    if country_registry.get(new_owner).is_none() {
        return Err(OwnershipTransferError::CountryNotFound);
    }

    if state.owner_country_id != expected_current_owner {
        return Err(OwnershipTransferError::CurrentOwnerMismatch);
    }

    if expected_current_owner == new_owner {
        return Err(OwnershipTransferError::SameOwner);
    }

    let capital_reassignment = plan_capital_reassignment(
        state_id,
        expected_current_owner,
        state_registry,
        country_registry,
    )?;

    Ok(ValidatedOwnershipTransfer {
        state_id,
        previous_owner: expected_current_owner,
        new_owner,
        capital_reassignment,
    })
}

/// P21-021: 国家消滅を伴う「最後の所有陸州」移転専用の事前検証。
///
/// `preflight_transfer_state_ownership`と同じ§3の基本validation(State存在・
/// 非海州・両国存在・現在所有者一致・新旧所有者相違)を行うが、**首都再配置は
/// 一切試みない**——旧所有国はこの移転で消滅するため、「消滅する国の新首都」
/// という概念自体が成立しない(`country::elimination`側が別途、国家状態を
/// `Eliminated`へ遷移させる)。呼び出し元(`country::elimination::
/// preflight_country_elimination`)が、通常の`preflight_transfer_state_
/// ownership`が`LastOwnedLandState`を返した場合にのみこちらを使う。
pub fn preflight_final_state_transfer_for_elimination(
    state_id: StateId,
    expected_current_owner: CountryId,
    new_owner: CountryId,
    state_registry: &StateRegistry,
    country_registry: &CountryRegistry,
) -> Result<ValidatedOwnershipTransfer, OwnershipTransferError> {
    let state = state_registry
        .get(state_id)
        .ok_or(OwnershipTransferError::StateNotFound)?;

    if state.is_sea {
        return Err(OwnershipTransferError::SeaState);
    }
    if country_registry.get(expected_current_owner).is_none() {
        return Err(OwnershipTransferError::CountryNotFound);
    }
    if country_registry.get(new_owner).is_none() {
        return Err(OwnershipTransferError::CountryNotFound);
    }
    if state.owner_country_id != expected_current_owner {
        return Err(OwnershipTransferError::CurrentOwnerMismatch);
    }
    if expected_current_owner == new_owner {
        return Err(OwnershipTransferError::SameOwner);
    }

    Ok(ValidatedOwnershipTransfer {
        state_id,
        previous_owner: expected_current_owner,
        new_owner,
        capital_reassignment: None,
    })
}

/// P21-021: `preflight_final_state_transfer_for_elimination`で検証した計画を
/// 適用する。TOCTOU対策として、適用直前に同じ(首都再配置を試みない)専用
/// preflightを再実行してから適用する(`apply_ownership_transfer`が通常版の
/// preflightで再検証するのとは意図的に別関数に分けている——通常版で
/// 再検証すると、首都再配置候補が無いことを理由に`LastOwnedLandState`へ
/// 逆戻りしてしまうため)。
pub fn apply_final_state_transfer_for_elimination(
    plan: ValidatedOwnershipTransfer,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
) -> Result<OwnershipTransferOutcome, OwnershipTransferError> {
    let fresh = preflight_final_state_transfer_for_elimination(
        plan.state_id,
        plan.previous_owner,
        plan.new_owner,
        state_registry,
        country_registry,
    )?;

    let _ = state_registry.transfer_region_ownership_unchecked(fresh.state_id, fresh.new_owner);

    Ok(OwnershipTransferOutcome {
        state_id: fresh.state_id,
        previous_owner: fresh.previous_owner,
        new_owner: fresh.new_owner,
        capital_reassignment: None,
    })
}

/// 検証済み計画を適用する。
///
/// P21-020(ユーザー指摘対応、TOCTOU対策): `plan`を無条件に信頼せず、適用直前に
/// **同じ条件を`preflight_transfer_state_ownership`で再検証する**。呼び出し元が
/// `preflight`→(他の変更)→`apply`という2段階を経る間にRegistryが変化した場合
/// (例: 別の講和が同じStateを既に移転させていた等)、古い計画のまま無条件に
/// `transfer_region_ownership_unchecked`を適用すると、検証をすり抜けた不整合な
/// 書き込みが発生し得る。再検証によって、古い`plan`が保持していた首都再配置候補も
/// 含め常に最新のRegistry状態を基準に計算し直すため、"stale plan"を安全に拒否できる
/// (失敗時はRegistryを一切変更しない)。
///
/// 1. 必要なら旧所有国の`capital_state_id`を更新
/// 2. Stateの`owner_country_id`を新所有国へ変更(内部実装は既存の低レベルAPI
///    `StateRegistry::transfer_region_ownership_unchecked`をそのまま再利用し、
///    controller・占領進捗・原所有者・war_idの正常化ロジックを重複させない)
pub fn apply_ownership_transfer(
    plan: ValidatedOwnershipTransfer,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
) -> Result<OwnershipTransferOutcome, OwnershipTransferError> {
    // stale planを信頼せず、適用直前に同じ条件を再検証し、その結果(fresh)だけを使う。
    let fresh = preflight_transfer_state_ownership(
        plan.state_id,
        plan.previous_owner,
        plan.new_owner,
        state_registry,
        country_registry,
    )?;

    if let Some((owner, new_capital)) = fresh.capital_reassignment
        && let Some(c) = country_registry.get_mut(owner)
    {
        c.capital_state_id = new_capital;
    }

    // 直前のpreflightで既に「Stateが存在し海州ではない」ことを検証済みのため、
    // ここでの`transfer_region_ownership_unchecked`の失敗は構造的に起こり得ない。
    let _ = state_registry.transfer_region_ownership_unchecked(fresh.state_id, fresh.new_owner);

    Ok(OwnershipTransferOutcome {
        state_id: fresh.state_id,
        previous_owner: fresh.previous_owner,
        new_owner: fresh.new_owner,
        capital_reassignment: fresh.capital_reassignment,
    })
}

/// `preflight_transfer_state_ownership` + `apply_ownership_transfer`をまとめて行う
/// 簡易API。検証と適用の間に他の変更(講和のClaim消費・War終結など)を挟む必要が
/// ない単純な呼び出し元(例: P21-011の要求受諾)向け。
pub fn transfer_state_ownership(
    state_id: StateId,
    expected_current_owner: CountryId,
    new_owner: CountryId,
    state_registry: &mut StateRegistry,
    country_registry: &mut CountryRegistry,
) -> Result<OwnershipTransferOutcome, OwnershipTransferError> {
    let plan = preflight_transfer_state_ownership(
        state_id,
        expected_current_owner,
        new_owner,
        state_registry,
        country_registry,
    )?;
    apply_ownership_transfer(plan, state_registry, country_registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::CountryData;
    use crate::state::data::StateData;

    fn country(id: usize, capital: usize) -> CountryData {
        CountryData {
            id: CountryId(id),
            capital_state_id: StateId(capital),
            ..Default::default()
        }
    }

    fn land_state(id: usize, owner: usize) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            is_sea: false,
            ..Default::default()
        }
    }

    fn sea_state(id: usize, owner: usize) -> StateData {
        StateData {
            id: StateId(id),
            owner_country_id: CountryId(owner),
            is_sea: true,
            ..Default::default()
        }
    }

    fn registries(
        states: Vec<StateData>,
        countries: Vec<CountryData>,
    ) -> (StateRegistry, CountryRegistry) {
        let state_registry = StateRegistry::build(states);
        let country_registry = CountryRegistry { countries };
        (state_registry, country_registry)
    }

    // ─── 1. 正常な非首都州移転 ───────────────────────────────────────────
    #[test]
    fn normal_non_capital_transfer_succeeds() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 0), land_state(2, 1)],
            vec![country(0, 0), country(1, 2)],
        );
        let outcome =
            transfer_state_ownership(StateId(1), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap();
        assert_eq!(outcome.new_owner, CountryId(1));
        assert!(outcome.capital_reassignment.is_none());
        assert_eq!(sr.get(StateId(1)).unwrap().owner_country_id, CountryId(1));
        assert_eq!(cr.get(CountryId(0)).unwrap().capital_state_id, StateId(0));
    }

    // ─── 2. 正常な首都州移転と首都再配置 ─────────────────────────────────
    #[test]
    fn capital_transfer_reassigns_capital() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 0), land_state(2, 1)],
            vec![country(0, 0), country(1, 2)],
        );
        let outcome =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap();
        assert_eq!(
            outcome.capital_reassignment,
            Some((CountryId(0), StateId(1)))
        );
        assert_eq!(cr.get(CountryId(0)).unwrap().capital_state_id, StateId(1));
    }

    // ─── 3. 新首都が最小StateIdになる ───────────────────────────────────
    #[test]
    fn new_capital_is_lowest_remaining_state_id() {
        let (mut sr, mut cr) = registries(
            vec![
                land_state(5, 0),
                land_state(3, 0),
                land_state(7, 0),
                land_state(9, 1),
            ],
            vec![country(0, 5), country(1, 9)],
        );
        let outcome =
            transfer_state_ownership(StateId(5), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap();
        assert_eq!(
            outcome.capital_reassignment,
            Some((CountryId(0), StateId(3)))
        );
    }

    // ─── 4. State挿入順を変えても新首都が同じ ───────────────────────────
    #[test]
    fn new_capital_choice_is_independent_of_state_insertion_order() {
        let (mut sr_a, mut cr_a) = registries(
            vec![
                land_state(7, 0),
                land_state(3, 0),
                land_state(5, 0),
                land_state(9, 1),
            ],
            vec![country(0, 7), country(1, 9)],
        );
        let (mut sr_b, mut cr_b) = registries(
            vec![
                land_state(5, 0),
                land_state(9, 1),
                land_state(7, 0),
                land_state(3, 0),
            ],
            vec![country(0, 7), country(1, 9)],
        );
        let a =
            transfer_state_ownership(StateId(7), CountryId(0), CountryId(1), &mut sr_a, &mut cr_a)
                .unwrap();
        let b =
            transfer_state_ownership(StateId(7), CountryId(0), CountryId(1), &mut sr_b, &mut cr_b)
                .unwrap();
        assert_eq!(a.capital_reassignment, b.capital_reassignment);
        assert_eq!(a.capital_reassignment, Some((CountryId(0), StateId(3))));
    }

    // ─── 5. controllerではなくownerを使う ───────────────────────────────
    #[test]
    fn capital_candidate_selection_uses_owner_not_controller() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 0), land_state(2, 1)],
            vec![country(0, 0), country(1, 2)],
        );
        // State(1)は自国所有だが、Controllerだけ第三国(1)に占領されている状態。
        sr.get_mut(StateId(1)).unwrap().controller_country = Some(CountryId(1));
        let outcome =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap();
        // 占領中でも法的owner=0なので依然として有効な首都候補になる。
        assert_eq!(
            outcome.capital_reassignment,
            Some((CountryId(0), StateId(1)))
        );
    }

    // ─── 6. 海州を拒否 ──────────────────────────────────────────────────
    #[test]
    fn sea_state_target_is_rejected() {
        let (mut sr, mut cr) = registries(
            vec![sea_state(0, 0), land_state(1, 1)],
            vec![country(0, 0), country(1, 1)],
        );
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::SeaState);
    }

    // ─── 7. State不存在を拒否 ───────────────────────────────────────────
    #[test]
    fn nonexistent_state_is_rejected() {
        let (mut sr, mut cr) =
            registries(vec![land_state(0, 0)], vec![country(0, 0), country(1, 0)]);
        let err =
            transfer_state_ownership(StateId(999), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::StateNotFound);
    }

    // ─── 8. expected owner不存在を拒否 ──────────────────────────────────
    #[test]
    fn nonexistent_expected_owner_is_rejected() {
        let (mut sr, mut cr) = registries(vec![land_state(0, 0)], vec![country(0, 0)]);
        let err =
            transfer_state_ownership(StateId(0), CountryId(999), CountryId(0), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::CountryNotFound);
    }

    // ─── 9. new owner不存在を拒否 ───────────────────────────────────────
    #[test]
    fn nonexistent_new_owner_is_rejected() {
        let (mut sr, mut cr) = registries(vec![land_state(0, 0)], vec![country(0, 0)]);
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(999), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::CountryNotFound);
    }

    // ─── 10. 現在所有者不一致を拒否 ─────────────────────────────────────
    #[test]
    fn current_owner_mismatch_is_rejected() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0)],
            vec![country(0, 0), country(1, 0), country(2, 0)],
        );
        let err = transfer_state_ownership(
            StateId(0),
            CountryId(1), // 実際の所有者は0
            CountryId(2),
            &mut sr,
            &mut cr,
        )
        .unwrap_err();
        assert_eq!(err, OwnershipTransferError::CurrentOwnerMismatch);
    }

    // ─── 11. 同一所有者への移転を安全に拒否 ─────────────────────────────
    #[test]
    fn transfer_to_same_owner_is_rejected() {
        let (mut sr, mut cr) = registries(vec![land_state(0, 0)], vec![country(0, 0)]);
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(0), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::SameOwner);
    }

    // ─── 12. 最後の所有陸州移転を拒否 ───────────────────────────────────
    #[test]
    fn last_owned_land_state_transfer_is_rejected() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 1)],
            vec![country(0, 0), country(1, 1)],
        );
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::LastOwnedLandState);
    }

    // ─── 13. 残存州が海州だけなら拒否 ───────────────────────────────────
    #[test]
    fn only_remaining_state_being_sea_is_rejected() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), sea_state(1, 0), land_state(2, 1)],
            vec![country(0, 0), country(1, 2)],
        );
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::LastOwnedLandState);
    }

    // ─── 14. 首都再配置候補に第三国所有州を使わない ─────────────────────
    #[test]
    fn capital_candidate_never_uses_third_party_owned_state() {
        let (mut sr, mut cr) = registries(
            vec![
                land_state(0, 0),
                land_state(1, 9), // 第三国所有、State(0)より小さいIdだが候補外
                land_state(2, 0),
                land_state(3, 1),
            ],
            vec![country(0, 0), country(1, 3), country(9, 1)],
        );
        let outcome =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap();
        assert_eq!(
            outcome.capital_reassignment,
            Some((CountryId(0), StateId(2)))
        );
    }

    // ─── 15. 失敗時にState Registry不変 ─────────────────────────────────
    #[test]
    fn failure_leaves_state_registry_unchanged() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 1)],
            vec![country(0, 0), country(1, 1)],
        );
        let before = format!("{:?}", sr.states);
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::LastOwnedLandState);
        assert_eq!(format!("{:?}", sr.states), before);
    }

    // ─── 16. 失敗時にCountry Registry不変 ───────────────────────────────
    #[test]
    fn failure_leaves_country_registry_unchanged() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 1)],
            vec![country(0, 0), country(1, 1)],
        );
        let before = format!("{:?}", cr.countries);
        let err =
            transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &mut sr, &mut cr)
                .unwrap_err();
        assert_eq!(err, OwnershipTransferError::LastOwnedLandState);
        assert_eq!(format!("{:?}", cr.countries), before);
    }

    // ─── 17. 成功時に無関係な国家・州を変更しない ───────────────────────
    #[test]
    fn success_does_not_touch_unrelated_states_or_countries() {
        let (mut sr, mut cr) = registries(
            vec![
                land_state(0, 0),
                land_state(1, 0),
                land_state(2, 1),
                land_state(9, 5), // 無関係な第三国の州
            ],
            vec![country(0, 0), country(1, 2), country(5, 9)],
        );
        let before_unrelated_state = format!("{:?}", sr.get(StateId(9)).unwrap());
        let before_unrelated_country = format!("{:?}", cr.get(CountryId(5)).unwrap());
        transfer_state_ownership(StateId(1), CountryId(0), CountryId(1), &mut sr, &mut cr).unwrap();
        assert_eq!(
            format!("{:?}", sr.get(StateId(9)).unwrap()),
            before_unrelated_state
        );
        assert_eq!(
            format!("{:?}", cr.get(CountryId(5)).unwrap()),
            before_unrelated_country
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // P21-020(ユーザー指摘対応): TOCTOU対策——`apply_ownership_transfer`が
    // stale(preflight後にRegistryが変化した)計画を無条件に信頼しないことの
    // 検証(§14要求)。
    // ─────────────────────────────────────────────────────────────────────

    /// preflight成功後、apply前に対象州の所有者が第三者へ変わった場合、
    /// applyは古い計画を無条件に適用せず、再検証して`CurrentOwnerMismatch`を
    /// 返す(Registryは一切変更しない)。
    #[test]
    fn apply_rejects_a_stale_plan_when_the_owner_changed_after_preflight() {
        let (mut sr, mut cr) = registries(
            vec![land_state(0, 0), land_state(1, 0), land_state(2, 1)],
            vec![country(0, 0), country(1, 2)],
        );
        let plan =
            preflight_transfer_state_ownership(StateId(1), CountryId(0), CountryId(1), &sr, &cr)
                .unwrap();

        // preflightとapplyの間に、別経路で対象州の所有者が変わったと仮定する
        // (例: 別の講和が先にこの州を奪った)。
        sr.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(2);
        cr.countries.push(country(2, 999));

        let before_states = format!("{:?}", sr.states);
        let before_countries = format!("{:?}", cr.countries);

        let err = apply_ownership_transfer(plan, &mut sr, &mut cr).unwrap_err();

        assert_eq!(err, OwnershipTransferError::CurrentOwnerMismatch);
        assert_eq!(
            format!("{:?}", sr.states),
            before_states,
            "a stale plan must not mutate the state registry"
        );
        assert_eq!(
            format!("{:?}", cr.countries),
            before_countries,
            "a stale plan must not mutate the country registry"
        );
    }

    /// preflight成功後(首都再配置候補を含む)、apply前にその再配置候補州の
    /// 所有者が変わった場合、applyは古い(無効になった)候補を使わず、
    /// 適用直前に再計算した新しい候補を使う。
    #[test]
    fn apply_recomputes_capital_candidate_when_it_changed_after_preflight() {
        let (mut sr, mut cr) = registries(
            vec![
                land_state(0, 0), // 首都、割譲対象
                land_state(1, 0), // preflight時点の再配置候補(最小StateId)
                land_state(2, 0), // 2番目の候補
                land_state(9, 1),
            ],
            vec![country(0, 0), country(1, 9)],
        );
        let plan =
            preflight_transfer_state_ownership(StateId(0), CountryId(0), CountryId(1), &sr, &cr)
                .unwrap();
        assert_eq!(
            plan.capital_reassignment(),
            Some((CountryId(0), StateId(1))),
            "precondition: preflight must have chosen State(1) as the candidate"
        );

        // preflightとapplyの間に、その候補州(State 1)が別経路で第三国へ
        // 渡ったと仮定する。stale planをそのまま適用すると、もはや自国の
        // 所有ではないState 1を新首都にしてしまう(壊れた不変条件)。
        sr.get_mut(StateId(1)).unwrap().owner_country_id = CountryId(9);

        let outcome = apply_ownership_transfer(plan, &mut sr, &mut cr).unwrap();

        assert_eq!(
            outcome.capital_reassignment,
            Some((CountryId(0), StateId(2))),
            "apply must recompute the capital candidate fresh, skipping the now-invalid \
             State(1) and picking the next valid one (State 2), not blindly reuse the stale \
             candidate"
        );
        assert_eq!(cr.get(CountryId(0)).unwrap().capital_state_id, StateId(2));
    }
}
