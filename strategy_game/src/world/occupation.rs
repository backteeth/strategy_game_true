//! P32-036 (M6-c): プロヴィンス単位の**即時占領**と実効支配の更新(純粋な Rust)。
//!
//! **D8(占領方式)の判断**: 旧ゲーム本体の挙動 = **即時占領**を採用する(守備隊のいない敵支配地への到着、または戦闘での攻撃側の勝利で、その場で実効支配が変わる。
//! 旧の本番経路は `invasion::occupy_state` が `controller_country` を書き換えるだけで、進捗(`occupation_progress`)も `war_id` も書かない)。旧に `war/occupation.rs::process_occupation`
//! (師団1個あたり +5/日、100 で交代 = 20日)があるが、**本番のシステムから呼ばれない**。**+5/日の占領進捗を新経路に追加するかどうかは、将来の仕様判断として残す**(今回は決めない)。
//!
//! 実効支配の正本は `ProvinceControl`。**法的所有はステート単位のまま**(このモジュールはステートにも所有国にも触れない)。占領による変更は**対象プロヴィンスの実効支配だけ**で、
//! 同じステートの別プロヴィンスへは波及しない。
//!
//! ## 適用(`apply_claim`)と日次の中での位置
//! M6-b が出す更新要求(`ControlClaim`)を共通の処理で適用する。**守備隊がいない場合は到着処理の中で、戦闘に勝った場合は終了処理の中で、その場で適用する**(いずれも当日の組織率回復より前。
//! 旧は終了処理の中で支配を変えるので、同日の組織率回復が効く)。同日の後続の移動・接触・退路の判定は、それぞれの処理の時点の(更新済みの)支配国を参照する。
//!
//! 適用の前に、次の順に確認する(最初に当てはまったものを結果とし、**支配は上書きしない**):
//! 1. 現在の支配国が既に要求した国 → `AlreadyApplied`(適用済みの要求)
//! 2. 現在の支配国が、要求時の支配国と違う → `Stale`(支配国が変わったあとに届いた古い要求)
//! 3. 現在の支配国が支配国なし、または要求した国と交戦していない → `NotAtWar`
//! 4. 対象地点に、要求した国の有効な師団(存在し、所属が要求した国で、現在地が対象で、戦闘中でない)がいない → `NoValidAttacker`
//! 5. 対象地点に、要求した国と交戦中の国の師団が残っている(**戦闘に参加しなかった交戦国の師団**を含む) → `EnemyDivisionsRemain`(**見送り**。理由を明示し、**翌日に自動で再適用しない**)
//! 6. 上記のいずれでもなければ適用(`Applied`)。
//!
//! 同日・同地点の競合は、既存の安定した処理順(移動と接触は `MapDivisionId` 昇順、戦闘の終了処理は `MapBattleId` 昇順)で、先に適用された要求が勝つ。後から届く要求は 1 または 2 になる。
//!
//! 戦闘に参加しなかった交戦国でない師団(中立国・自国の師団)は、支配の更新を妨げず、**既存の状態を保つ**。
//!
//! ## M6-d への通知
//! 支配が実際に変わると `MoveEvent::ControlChanged { province, previous, new, reason, divisions }` が出る。`countries_to_invalidate(events)` が、1日分の出来事から
//! **到達結果を無効化すべき国家**(変更前の支配国と変更後の支配国。重複なし、国家番号の昇順)を返す。**無効化の通知までが今回の範囲**で、`Reach` のキャッシュと日次補給への接続は M6-d。

use super::ids::ProvinceId;
use super::movement::{ClaimReason, MapDivisionId, MapDivisionStatus, MapMilitary, MoveEvent};
use crate::common::CountryId;
use std::collections::BTreeMap;

/// 実効支配の更新要求(M6-b の `MoveEvent::ControlClaim` と同じ内容)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRequest {
    pub province: ProvinceId,
    pub claimant: CountryId,
    /// 要求を出した時点の支配国(古い要求の検出に使う)。
    pub previous_controller: Option<CountryId>,
    pub reason: ClaimReason,
    pub divisions: Vec<MapDivisionId>,
}

/// 適用しなかった理由(支配は上書きしない)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRejection {
    /// 現在の支配国が既に要求した国(適用済みの要求)。
    AlreadyApplied,
    /// 現在の支配国が、要求時の支配国と違う(支配国が変わったあとに届いた古い要求)。
    Stale { current: Option<CountryId>, requested_from: Option<CountryId> },
    /// 現在の支配国が支配国なし、または要求した国と交戦していない。
    NotAtWar { current: Option<CountryId> },
    /// 対象地点に、要求した国の有効な師団がいない。
    NoValidAttacker,
    /// 対象地点に、要求した国と交戦中の国の師団が残っている(見送り。自動では再適用しない)。
    EnemyDivisionsRemain { divisions: Vec<MapDivisionId> },
}

/// 適用の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Applied { previous: Option<CountryId>, new: CountryId },
    Rejected(ClaimRejection),
}

impl MapMilitary {
    /// 更新要求を適用する(上の確認の順)。適用したときだけ、対象プロヴィンスの実効支配を変える(ほかのプロヴィンス・ステート・所有国には触れない)。
    pub fn apply_claim(&mut self, req: &ClaimRequest) -> ClaimOutcome {
        let current = self.control.controller(req.province);
        if current == Some(req.claimant) {
            return ClaimOutcome::Rejected(ClaimRejection::AlreadyApplied);
        }
        if current != req.previous_controller {
            return ClaimOutcome::Rejected(ClaimRejection::Stale { current, requested_from: req.previous_controller });
        }
        match current {
            Some(c) if self.at_war(req.claimant, c) => {}
            _ => return ClaimOutcome::Rejected(ClaimRejection::NotAtWar { current }),
        }
        let valid_attacker = req
            .divisions
            .iter()
            .any(|id| self.divisions.get(id).is_some_and(|d| d.owner == req.claimant && d.province == req.province && d.status != MapDivisionStatus::Fighting));
        if !valid_attacker {
            return ClaimOutcome::Rejected(ClaimRejection::NoValidAttacker);
        }
        let enemies: Vec<MapDivisionId> = self.divisions.values().filter(|d| d.province == req.province && self.at_war(req.claimant, d.owner)).map(|d| d.id).collect();
        if !enemies.is_empty() {
            return ClaimOutcome::Rejected(ClaimRejection::EnemyDivisionsRemain { divisions: enemies });
        }
        self.control.set(req.province, Some(req.claimant));
        // 到達結果(M6-d)の無効化: 変更前と変更後の支配国。再計算は次に補給を計画する時点で行う。
        let affected: Vec<CountryId> = current.into_iter().chain(std::iter::once(req.claimant)).collect();
        self.invalidate_reach(&affected);
        ClaimOutcome::Applied { previous: current, new: req.claimant }
    }

    /// 日次処理の中の共通処理: 要求(`ControlClaim`)を出し、その場で適用して、結果(`ControlChanged` または `ClaimNotApplied`)を出す。
    pub(super) fn claim_and_report(&mut self, req: ClaimRequest, events: &mut Vec<MoveEvent>) {
        events.push(MoveEvent::ControlClaim {
            province: req.province,
            claimant: req.claimant,
            previous_controller: req.previous_controller,
            reason: req.reason,
            divisions: req.divisions.clone(),
        });
        match self.apply_claim(&req) {
            ClaimOutcome::Applied { previous, new } => events.push(MoveEvent::ControlChanged { province: req.province, previous, new, reason: req.reason, divisions: req.divisions }),
            ClaimOutcome::Rejected(rejection) => events.push(MoveEvent::ClaimNotApplied { province: req.province, claimant: req.claimant, reason: req.reason, rejection }),
        }
    }
}

/// 1日分の出来事から、到達結果を無効化すべき国家(支配が変わったプロヴィンスの、変更前の支配国と変更後の支配国)を返す。重複なし、国家番号の昇順。
/// 支配国なしから変わった場合の「変更前」は含まれない(国家がない)。M6-d が `Reach` のキャッシュの無効化に使う。
pub fn countries_to_invalidate(events: &[MoveEvent]) -> Vec<CountryId> {
    let mut set: BTreeMap<usize, CountryId> = BTreeMap::new();
    for e in events {
        if let MoveEvent::ControlChanged { previous, new, .. } = e {
            if let Some(p) = previous {
                set.insert(p.0, *p);
            }
            set.insert(new.0, *new);
        }
    }
    set.into_values().collect()
}
